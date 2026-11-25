//! Owns: the list Word gives a built-in list style on its first use (docx-agent's
//! `builtin_styles.write` with `numbering.add_abstract` and `add_num`): the measured
//! abstract definition numbered after the document's last, its `w:nsid` and `w:tmpl` drawn
//! as docx-agent draws them (`_nsid`: a SHA-256 of the style's seed, past the ones in use),
//! put after the last abstract definition, and an instance of it after the last instance;
//! the numbering part made first when the document has none.
//!
//! Must not: write but through the sink, or change a definition or instance already there.

use crate::doc::Key;
use crate::names::W;
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_xml::{Attribute, Element, Head, Limits, Node, QName};
use std::collections::HashSet;
use std::sync::Arc;

/// The numbering part's content type.
const NUMBERING_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
/// The relationship type of the numbering part.
const REL_NUMBERING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";

/// An element parsed from measured XML, its own namespace declarations dropped (the part's
/// root declares them, as lxml leaves an element moved under it).
pub(crate) fn parsed(xml: &str) -> Result<Arc<Element>> {
    let tree = ooxml_xml::parse(xml.as_bytes(), &Limits::default())
        .map_err(|e| EditError::Refused(format!("a measured definition does not parse: {e}")))?;
    let root = tree.root();
    let head = Head {
        name: root.name().clone(),
        namespaces: Box::new([]),
        attributes: root.attributes().into(),
    };
    Ok(Arc::new(Element::from_head(Arc::new(head), root.children().clone())))
}

/// The numbers the children `w:{local}` of `root` carry in `w:{attribute}`.
fn numbers(root: &Element, local: &str, attribute: &str) -> Vec<i64> {
    root.child_elements()
        .filter(|c| c.name().is(Some(W), local))
        .filter_map(|c| c.attribute(Some(W), attribute)?.trim().parse().ok())
        .collect()
}

/// The place of the last child `w:{local}` of `root`.
fn last_of(root: &Element, local: &str) -> Option<usize> {
    let children = root.children().to_vec();
    children
        .iter()
        .rposition(|n| matches!(n, Node::Element(e) if e.name().is(Some(W), local)))
}

/// The place of the first child `w:{local}` of `root`.
fn first_of(root: &Element, local: &str) -> Option<usize> {
    root.children()
        .iter()
        .position(|n| matches!(n, Node::Element(e) if e.name().is(Some(W), local)))
}

/// Every `w:nsid` value in use, in upper case.
fn nsids(element: &Element, out: &mut HashSet<String>) {
    for child in element.child_elements() {
        if child.name().is(Some(W), "nsid") {
            out.insert(child.attribute(Some(W), "val").unwrap_or("").to_uppercase());
        }
        nsids(child, out);
    }
}

/// docx-agent's `_nsid`: the first of SHA-256(`seed\0counter`)'s eight leading hex digits,
/// in upper case, not in use and not `00000000`.
fn nsid(seed: &str, used: &HashSet<String>) -> String {
    (0u64..)
        .map(|counter| {
            let digest = ooxml_word_model::ids::sha256(format!("{seed}\0{counter}").as_bytes());
            digest.iter().take(4).map(|b| format!("{b:02X}")).collect::<String>()
        })
        .find(|v| !used.contains(v) && v != "00000000")
        .unwrap_or_default()
}

/// Add the list of the style `style_id` (its measured abstract definition, `definition`)
/// to the numbering part; the new instance's id.
///
/// # Errors
/// When the numbering part is not XML.
pub(crate) fn add_list(sink: &mut EditSink<'_, '_>, main: &str, style_id: &str, definition: &str) -> Result<String> {
    let part = match ooxml_word_model::parts::related(sink.doc().package(), main, REL_NUMBERING)? {
        Some(part) => part,
        // docx-agent's `numbering._ensure_part`: made as Word declares it
        None => sink.new_part(
            main,
            ("word/numbering.xml", "word/numbering{n}.xml"),
            crate::new_part::word_root("numbering"),
            NUMBERING_TYPE,
            REL_NUMBERING,
        )?,
    };
    let key = || vec![Key::part(part.as_str())];
    sink.read(Key::part(part.as_str()));
    let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::no_part(&part))?;
    let root = tree.root();
    let abstract_id = numbers(root, "abstractNum", "abstractNumId")
        .into_iter()
        .max()
        .unwrap_or(-1)
        + 1;
    let seed = format!("style\0{style_id}\0{abstract_id}");
    let mut used = HashSet::new();
    nsids(root, &mut used);
    let xml = definition
        .replace("{abstract}", &abstract_id.to_string())
        .replace("{id}", style_id)
        .replace("{nsid}", &nsid(&format!("{seed}nsid"), &used))
        .replace("{tmpl}", &nsid(&format!("{seed}tmpl"), &used));
    let at = last_of(root, "abstractNum")
        .map(|k| k + 1)
        .or_else(|| first_of(root, "num"))
        .or_else(|| first_of(root, "numIdMacAtCleanup"))
        .unwrap_or(root.children().len());
    sink.splice_children(
        &part,
        (&[], at, 0),
        vec![Node::Element(parsed(&xml)?)],
        key(),
        Vec::new(),
    )?;
    let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::no_part(&part))?;
    let root = tree.root();
    let num_id = numbers(root, "num", "numId").into_iter().max().unwrap_or(0) + 1;
    let prefix = root.name().prefix().map(str::to_owned);
    let name = |local: &str| Arc::new(QName::new(prefix.as_deref(), local, Some(W)));
    let attribute = |local: &str, value: String| Attribute {
        name: name(local),
        value: Arc::from(value),
    };
    let reference = Element::from_head(
        Arc::new(Head {
            name: name("abstractNumId"),
            namespaces: Box::new([]),
            attributes: vec![attribute("val", abstract_id.to_string())].into(),
        }),
        ooxml_xml::Children::from_vec(Vec::new()),
    );
    let num = Element::from_head(
        Arc::new(Head {
            name: name("num"),
            namespaces: Box::new([]),
            attributes: vec![attribute("numId", num_id.to_string())].into(),
        }),
        ooxml_xml::Children::from_vec(vec![Node::Element(Arc::new(reference))]),
    );
    let at = last_of(root, "num")
        .map(|k| k + 1)
        .or_else(|| first_of(root, "numIdMacAtCleanup"))
        .unwrap_or(root.children().len());
    sink.splice_children(
        &part,
        (&[], at, 0),
        vec![Node::Element(Arc::new(num))],
        key(),
        Vec::new(),
    )?;
    Ok(num_id.to_string())
}

/// `element` with every `w:numId` below it that reads `{num}` given `num`.
pub(crate) fn with_num(element: &Arc<Element>, num: &str) -> Arc<Element> {
    let mut changed = false;
    let children: Vec<Node> = element
        .children()
        .iter()
        .map(|node| match node {
            Node::Element(child) => {
                let mut new = with_num(child, num);
                if child.name().is(Some(W), "numId") && child.attribute(Some(W), "val") == Some("{num}") {
                    let val = child
                        .attributes()
                        .iter()
                        .find(|a| a.name.is(Some(W), "val"))
                        .map(|a| Arc::clone(&a.name));
                    if let Some(val) = val {
                        let head = crate::stamping::with_attribute(new.head(), &val, num);
                        new = Arc::new(Element::from_head(Arc::new(head), new.children().clone()));
                    }
                }
                changed |= !Arc::ptr_eq(&new, child);
                Node::Element(new)
            }
            other => other.clone(),
        })
        .collect();
    if changed {
        Arc::new(element.with_children(ooxml_xml::Children::from_vec(children)))
    } else {
        Arc::clone(element)
    }
}
