//! Owns: a data-bound content control's custom XML node (docx-agent's `_bound_node`,
//! `_write_bound`): the custom XML part its `w:dataBinding` names by store item id (the
//! only one, where it names none), the node its XPath finds there, and that node written
//! with the value the control shows -- Word fills a bound control from its node on opening,
//! so a fill writes both (as docx4j does).
//!
//! The XPath Word writes is a path of child steps from the root, each a name and a
//! position (`/ns0:root[1]/ns0:client[1]`); that is what is evaluated here. Any other
//! expression is left to the host (`EditError::Unsupported`) rather than read differently.
//!
//! Must not: make a node the binding names but the part lacks, or write but through the
//! sink.

use crate::doc::Key;
use crate::sink::EditSink;
use crate::{EditError, Result, WordDoc};
use ooxml_base::pyfmt::repr_str;
use ooxml_word_model::docx::controls::property;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::W;
use ooxml_word_model::parts::related_parts;
use ooxml_xml::{Atom, Element, Node};
use std::sync::Arc;

const REL_CUSTOM_XML: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml";
const REL_CUSTOM_XML_PROPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXmlProps";
const DS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/customXml";

/// A control's binding: its XPath, store item id and prefix mappings.
pub(crate) struct Binding {
    xpath: String,
    store: Option<String>,
    prefixes: Vec<(String, String)>,
}

impl Binding {
    /// The binding of `sdt`, if it has one.
    pub(crate) fn of(sdt: &Element) -> Option<Binding> {
        let node = property(sdt, W, "dataBinding")?;
        let text = |local: &str| node.attribute(Some(W), local).map(str::to_owned);
        Some(Binding {
            xpath: text("xpath").unwrap_or_default(),
            store: text("storeItemID"),
            prefixes: prefixes(&text("prefixMappings").unwrap_or_default()),
        })
    }

    /// The custom XML part and the place (child indexes from its root) of the node the
    /// binding names (`_bound_node`).
    ///
    /// # Errors
    /// `refused` for a missing part or node; `unsupported` for an XPath not read here.
    pub(crate) fn node(&self, doc: &WordDoc, docx: &Docx) -> Result<(String, Vec<u32>)> {
        let stores = store_parts(doc, docx)?;
        let wanted = self.store.as_deref().unwrap_or("").to_uppercase();
        let part = match stores.iter().find(|(id, _)| *id == wanted) {
            Some((_, part)) => part.clone(),
            None if stores.len() == 1 && self.store.as_deref().is_none_or(str::is_empty) => {
                stores.first().map(|(_, p)| p.clone()).unwrap_or_default()
            }
            None => {
                return Err(EditError::Refused(format!(
                    "the custom XML part {} the control is bound to is missing",
                    self.store.as_deref().unwrap_or("None")
                )))
            }
        };
        let steps = steps(&self.xpath, &self.prefixes).ok_or_else(|| {
            EditError::Unsupported("word_controls fill of a control bound by an XPath other than child steps".into())
        })?;
        // the store's relationship names a part the package lacks: the document's, not ours
        let tree = doc.tree(&part)?.ok_or_else(|| {
            EditError::Refused(format!(
                "the custom XML part {} the control is bound to is missing",
                self.store.as_deref().unwrap_or("None")
            ))
        })?;
        find(tree.root(), &steps).map(|path| (part, path)).ok_or_else(|| {
            EditError::Refused(format!(
                "the bound node {} is not in the custom XML part (docx-agent writes a binding's node, it does not make one)",
                repr_str(&self.xpath)
            ))
        })
    }

    /// The text of the node the binding names, where it holds text alone (`None` where it
    /// holds an element or anything but text).
    ///
    /// # Errors
    /// As [`Binding::node`].
    pub(crate) fn text(&self, doc: &WordDoc, docx: &Docx) -> Result<Option<String>> {
        let (part, path) = self.node(doc, docx)?;
        let tree = doc.tree(&part)?.ok_or_else(|| EditError::no_part(part.clone()))?;
        let node =
            ooxml_word_model::docx::at_path(tree.root(), &path).ok_or_else(|| EditError::no_element(part.clone()))?;
        let mut text = String::new();
        for child in node.children().iter() {
            match child {
                Node::Text(t) => text.push_str(t),
                _ => return Ok(None),
            }
        }
        Ok(Some(text))
    }

    /// The node written with `value`, its children gone (`_write_bound`); nothing when it
    /// holds that text already.
    ///
    /// # Errors
    /// As [`Binding::node`].
    pub(crate) fn write(&self, sink: &mut EditSink<'_, '_>, value: &str) -> Result<()> {
        let docx = sink.docx()?;
        let (part, path) = self.node(sink.doc(), &docx)?;
        sink.read(Key::part(part.as_str()));
        if path.is_empty() {
            return Err(EditError::Unsupported(
                "word_controls fill of a control bound to a part's root".into(),
            ));
        }
        let tree = sink
            .doc()
            .tree(&part)?
            .ok_or_else(|| EditError::no_part(part.clone()))?;
        let node =
            ooxml_word_model::docx::at_path(tree.root(), &path).ok_or_else(|| EditError::no_element(part.clone()))?;
        // lxml's `.text`: `None` where the node does not open with text (`<a/>` is written
        // `<a></a>` once given "")
        let text = match node.children().iter().next() {
            Some(Node::Text(t)) => Some(t.as_ref()),
            _ => None,
        };
        if text == Some(value) {
            return Ok(());
        }
        let new = Arc::new(Element::new(
            node.name().clone(),
            node.namespaces().to_vec(),
            node.attributes().to_vec(),
            vec![Node::Text(Atom::from(value))],
        ));
        sink.replace(&part, &path, new, vec![Key::part(part.as_str())])
    }

    /// The node written with `value` where it holds no element (or other node but text)
    /// and other text (`_sync_bound_controls`' rule); a node holding elements is left.
    ///
    /// # Errors
    /// As [`Binding::node`] (the caller leaves a control whose part or node is missing).
    pub(crate) fn sync(&self, sink: &mut EditSink<'_, '_>, docx: &Docx, value: &str) -> Result<()> {
        let (part, path) = self.node(sink.doc(), docx)?;
        let tree = sink
            .doc()
            .tree(&part)?
            .ok_or_else(|| EditError::no_part(part.clone()))?;
        let node =
            ooxml_word_model::docx::at_path(tree.root(), &path).ok_or_else(|| EditError::no_element(part.clone()))?;
        if node.children().iter().any(|n| !matches!(n, Node::Text(_))) {
            return Ok(());
        }
        self.write(sink, value)
    }
}

/// `{prefix: namespace}` of a binding's `w:prefixMappings` (`xmlns:ns0='urn:x'`).
fn prefixes(mappings: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = mappings;
    while let Some(at) = rest.find("xmlns:") {
        rest = &rest[at + 6..];
        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let after = rest[name.len()..].trim_start();
        let Some(after) = after.strip_prefix('=') else { continue };
        let after = after.trim_start();
        let Some(quote) = after.chars().next().filter(|q| *q == '\'' || *q == '"') else {
            continue;
        };
        let Some(end) = after[1..].find(quote) else { break };
        out.push((name, after[1..=end].to_owned()));
        rest = &after[end + 2..];
    }
    out
}

/// A child step: the namespace and local name, and its 1-based position if given.
type Step = (Option<String>, String, Option<usize>);

/// The steps of an XPath of child steps from the root (`/a:b[1]/a:c`), or `None` for
/// any other expression.
fn steps(xpath: &str, prefixes: &[(String, String)]) -> Option<Vec<Step>> {
    let rest = xpath.trim().strip_prefix('/')?;
    rest.split('/')
        .map(|step| {
            let (name, position) = match step.split_once('[') {
                Some((name, index)) => (name, Some(index.strip_suffix(']')?.trim().parse::<usize>().ok()?)),
                None => (step, None),
            };
            let (namespace, local) = match name.split_once(':') {
                Some((prefix, local)) => (Some(prefixes.iter().find(|(p, _)| p == prefix)?.1.clone()), local),
                None => (None, name),
            };
            let simple = !local.is_empty() && local.chars().all(|c| c.is_alphanumeric() || "_-.".contains(c));
            simple.then(|| (namespace, local.to_owned(), position))
        })
        .collect()
}

/// The place of the first element the steps find from the document node.
fn find(root: &Arc<Element>, steps: &[Step]) -> Option<Vec<u32>> {
    let matches = |e: &Element, (namespace, local, _): &Step| e.name().is(namespace.as_deref(), local);
    let (first, rest) = steps.split_first()?;
    if !matches(root, first) || first.2.is_some_and(|n| n != 1) {
        return None;
    }
    let mut found: Vec<(Vec<u32>, &Arc<Element>)> = vec![(Vec::new(), root)];
    for step in rest {
        let mut next = Vec::new();
        for (path, element) in &found {
            let children = element.children().iter().enumerate().filter_map(|(k, n)| match n {
                Node::Element(e) if matches(e, step) => Some((k, e)),
                _ => None,
            });
            for (n, (k, child)) in children.enumerate() {
                if step.2.is_none_or(|want| want == n + 1) {
                    let mut at = path.clone();
                    at.push(u32::try_from(k).ok()?);
                    next.push((at, child));
                }
            }
        }
        found = next;
    }
    found.into_iter().next().map(|(path, _)| path)
}

/// `(store item id in upper case, custom XML part)` of each custom XML part of the main
/// part (`_store_parts`), in relationship order.
fn store_parts(doc: &WordDoc, docx: &Docx) -> Result<Vec<(String, String)>> {
    let package = doc.package();
    let mut out: Vec<(String, String)> = Vec::new();
    for part in related_parts(package, &docx.main, REL_CUSTOM_XML)? {
        if !package.has_part(&part) {
            continue;
        }
        for props in related_parts(package, &part, REL_CUSTOM_XML_PROPS)? {
            let Some(tree) = package
                .has_part(&props)
                .then(|| doc.tree(&props))
                .transpose()?
                .flatten()
            else {
                continue;
            };
            if let Some(id) = tree.root().attribute(Some(DS), "itemID").filter(|i| !i.is_empty()) {
                let id = id.to_uppercase();
                match out.iter_mut().find(|(k, _)| *k == id) {
                    Some(slot) => slot.1.clone_from(&part),
                    None => out.push((id, part.clone())),
                }
            }
        }
    }
    Ok(out)
}
