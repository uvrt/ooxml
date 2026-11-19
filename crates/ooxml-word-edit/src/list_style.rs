//! Owns: the list Word gives a built-in list style on its first use (docx-agent's
//! `builtin_styles.write` with `numbering.add_abstract` and `add_num`): the measured
//! abstract definition numbered after the document's last, its `w:nsid` and `w:tmpl` drawn
//! as docx-agent draws them (`_nsid`: a SHA-256 of the style's seed, past the ones in use),
//! put after the last abstract definition, and an instance of it after the last instance;
//! the numbering part made first when the document has none; and the numbering writes
//! insert_markdown's lists share (`add_abstract`, `add_num` with a level's start
//! override, `_nsid`).
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

/// The numbering part, made as Word declares it when the document has none (docx-agent's
/// `numbering._ensure_part`); read by the edit.
///
/// # Errors
/// When the relationships or content types are not XML.
pub(crate) fn numbering_part(sink: &mut EditSink<'_, '_>, main: &str) -> Result<String> {
    let part = match ooxml_word_model::parts::related(sink.doc().package(), main, REL_NUMBERING)? {
        Some(part) => part,
        None => sink.new_part(
            main,
            ("word/numbering.xml", "word/numbering{n}.xml"),
            crate::new_part::word_root("numbering"),
            NUMBERING_TYPE,
            REL_NUMBERING,
        )?,
    };
    sink.read(Key::Part(Arc::from(part.as_str())));
    Ok(part)
}

/// The numbering part's root now.
fn numbering_root(sink: &EditSink<'_, '_>, part: &str) -> Result<Arc<Element>> {
    let tree = sink
        .doc()
        .tree(part)?
        .ok_or_else(|| EditError::NoPart(part.to_owned()))?;
    Ok(Arc::clone(tree.root()))
}

/// The id the next abstract definition takes: one past the largest.
pub(crate) fn next_abstract_id(sink: &EditSink<'_, '_>, part: &str) -> Result<i64> {
    let root = numbering_root(sink, part)?;
    Ok(numbers(&root, "abstractNum", "abstractNumId")
        .into_iter()
        .max()
        .unwrap_or(-1)
        + 1)
}

/// docx-agent's `_nsid` over the numbering part as it is now.
pub(crate) fn nsid_for(sink: &EditSink<'_, '_>, part: &str, seed: &str) -> Result<String> {
    let mut used = HashSet::new();
    nsids(numbering_root(sink, part)?.as_ref(), &mut used);
    Ok(nsid(seed, &used))
}

/// docx-agent's `add_abstract`: `node` after the last abstract definition (before the
/// first instance, or `w:numIdMacAtCleanup`, else at the end).
///
/// # Errors
/// When the part is gone.
pub(crate) fn add_abstract(sink: &mut EditSink<'_, '_>, part: &str, node: Arc<Element>) -> Result<()> {
    let root = numbering_root(sink, part)?;
    let at = last_of(&root, "abstractNum")
        .map(|k| k + 1)
        .or_else(|| first_of(&root, "num"))
        .or_else(|| first_of(&root, "numIdMacAtCleanup"))
        .unwrap_or(root.children().len());
    let keys = vec![Key::Part(Arc::from(part))];
    sink.splice_children(part, (&[], at, 0), vec![Node::Element(node)], keys, Vec::new())
}

/// A `w:{local}` element named with `prefix`, its `w:` attributes and children.
pub(crate) fn w_element(
    prefix: Option<&str>,
    local: &str,
    attributes: &[(&str, String)],
    children: Vec<Node>,
) -> Element {
    let name = |local: &str| Arc::new(QName::new(prefix, local, Some(W)));
    let attributes: Vec<Attribute> = attributes
        .iter()
        .map(|(local, value)| Attribute {
            name: name(local),
            value: Arc::from(value.as_str()),
        })
        .collect();
    Element::from_head(
        Arc::new(Head {
            name: name(local),
            namespaces: Box::new([]),
            attributes: attributes.into(),
        }),
        ooxml_xml::Children::from_vec(children),
    )
}

/// docx-agent's `add_num`: a new instance over `abstract_id` after the last one, `override_`
/// a level's `w:startOverride` (and a whole `w:lvl` beside it); its numId.
///
/// # Errors
/// When the part is gone.
pub(crate) fn add_num(
    sink: &mut EditSink<'_, '_>,
    part: &str,
    abstract_id: i64,
    override_: Option<(i64, i64, Option<Arc<Element>>)>,
) -> Result<i64> {
    let root = numbering_root(sink, part)?;
    let num_id = numbers(&root, "num", "numId").into_iter().max().unwrap_or(0) + 1;
    let prefix = root.name().prefix().map(str::to_owned);
    let prefix = prefix.as_deref();
    let mut children = vec![Node::Element(Arc::new(w_element(
        prefix,
        "abstractNumId",
        &[("val", abstract_id.to_string())],
        Vec::new(),
    )))];
    if let Some((level, start, lvl)) = override_ {
        let mut inner = vec![Node::Element(Arc::new(w_element(
            prefix,
            "startOverride",
            &[("val", start.to_string())],
            Vec::new(),
        )))];
        inner.extend(lvl.map(Node::Element));
        children.push(Node::Element(Arc::new(w_element(
            prefix,
            "lvlOverride",
            &[("ilvl", level.to_string())],
            inner,
        ))));
    }
    let num = w_element(prefix, "num", &[("numId", num_id.to_string())], children);
    let at = last_of(&root, "num")
        .map(|k| k + 1)
        .or_else(|| first_of(&root, "numIdMacAtCleanup"))
        .unwrap_or(root.children().len());
    let keys = vec![Key::Part(Arc::from(part))];
    sink.splice_children(part, (&[], at, 0), vec![Node::Element(Arc::new(num))], keys, Vec::new())?;
    Ok(num_id)
}

/// Add the list of the style `style_id` (its measured abstract definition, `definition`)
/// to the numbering part; the new instance's id.
///
/// # Errors
/// When the numbering part is not XML.
pub(crate) fn add_list(sink: &mut EditSink<'_, '_>, main: &str, style_id: &str, definition: &str) -> Result<String> {
    let part = numbering_part(sink, main)?;
    let abstract_id = next_abstract_id(sink, &part)?;
    let seed = format!("style\0{style_id}\0{abstract_id}");
    let xml = definition
        .replace("{abstract}", &abstract_id.to_string())
        .replace("{id}", style_id)
        .replace("{nsid}", &nsid_for(sink, &part, &format!("{seed}nsid"))?)
        .replace("{tmpl}", &nsid_for(sink, &part, &format!("{seed}tmpl"))?);
    add_abstract(sink, &part, parsed(&xml)?)?;
    Ok(add_num(sink, &part, abstract_id, None)?.to_string())
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
