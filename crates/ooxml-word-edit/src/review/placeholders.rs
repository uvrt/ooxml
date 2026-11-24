//! Owns: what a content control a review empties shows, as Word 16.106's Accept All and
//! Reject All leave one (measured on made-up controls, fu-word3): `w:showingPlcHdr` set,
//! and one run in the control's run properties (`w:sdtPr/w:rPr`) holding the text of the
//! glossary building block its `w:placeholder` names, else five spaces. An inline control
//! takes the run in its content; a block-level one in the paragraph the review emptied.
//!
//! Must not: decide what the review removes (`super::part`).

use super::xml::{insert_in_order, make};
use crate::names::W;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{Result, WordDoc};
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// What a control shows when it holds nothing and names no placeholder text: five spaces
/// (Word 16.106's Reject All of `controls.docx`'s inserted-only control 309).
const SPACES: &str = "     ";

/// The relationship from the main document to its glossary (building blocks).
const REL_GLOSSARY: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument";

/// What a control's content shows: text, a tab or break, an object, a field, a deleted
/// text still pending.
const SHOWN: [&str; 17] = [
    "t",
    "delText",
    "tab",
    "ptab",
    "br",
    "cr",
    "sym",
    "noBreakHyphen",
    "softHyphen",
    "drawing",
    "pict",
    "object",
    "footnoteReference",
    "endnoteReference",
    "fldChar",
    "fldSimple",
    "instrText",
];

/// The placeholder texts of a document's glossary: each building block's name and the text
/// of its body.
pub(crate) type PlaceholderTexts = HashMap<String, String>;

/// The glossary's building blocks' texts, by name (none when the document has no glossary).
///
/// # Errors
/// When the relationships or the glossary part cannot be read.
pub(crate) fn placeholder_texts(doc: &WordDoc, main: &str) -> Result<PlaceholderTexts> {
    let package = doc.package();
    let Some(part) = ooxml_word_model::parts::related(package, main, REL_GLOSSARY)? else {
        return Ok(HashMap::new());
    };
    let Some(tree) = doc.tree(&part)? else {
        return Ok(HashMap::new());
    };
    let mut texts = HashMap::new();
    let Some(parts) = tree.root().find(Some(W), "docParts") else {
        return Ok(texts);
    };
    for block in parts.child_elements().filter(|e| e.name().is(Some(W), "docPart")) {
        let name = block
            .find(Some(W), "docPartPr")
            .and_then(|p| p.find(Some(W), "name"))
            .and_then(|n| n.attribute(Some(W), "val"));
        let body = block.find(Some(W), "docPartBody");
        if let (Some(name), Some(body)) = (name, body) {
            let mut text = String::new();
            texts_of(body, &mut text);
            texts.insert(name.to_owned(), text);
        }
    }
    Ok(texts)
}

/// The `w:t` text under `element`, in order.
fn texts_of(element: &Arc<Element>, into: &mut String) {
    for child in element.child_elements() {
        if child.name().is(Some(W), "t") {
            into.push_str(&child.text_content());
        } else {
            texts_of(child, into);
        }
    }
}

/// The content control whose content `record` stands directly in, and where its placeholder
/// run would go: an inline control's `w:sdtContent`, or the paragraph of a block-level
/// control's content that holds `record`.
pub(super) fn emptied_control(work: &Work, record: Id) -> Option<(Id, Id)> {
    let parent = work.parent(record)?;
    let (content, host) = if work.is(parent, W, "sdtContent") {
        (parent, parent)
    } else if work.is(parent, W, "p") {
        let content = work.parent(parent).filter(|c| work.is(*c, W, "sdtContent"))?;
        (content, parent)
    } else {
        return None;
    };
    let sdt = work.parent(content).filter(|s| work.is(*s, W, "sdt"))?;
    Some((sdt, host))
}

/// `sdt`, emptied by the review, shows its placeholder in `host` -- unless it still shows
/// something.
pub(super) fn show(work: &mut Work, orders: &Orders, (sdt, host): (Id, Id), texts: &PlaceholderTexts) {
    if work.parent(sdt).is_none() || work.parent(host).is_none() {
        return;
    }
    let (Some(properties), Some(content)) = (work.find(sdt, W, "sdtPr"), work.find(sdt, W, "sdtContent")) else {
        return;
    };
    let holds = work
        .descendants(content)
        .into_iter()
        .any(|n| work.is_any(n, W, &SHOWN) || foreign(work, n));
    if holds {
        return;
    }
    let named = work
        .find(properties, W, "placeholder")
        .and_then(|p| work.find(p, W, "docPart"))
        .and_then(|d| work.get(d, Some(W), "val"))
        .and_then(|name| texts.get(name))
        .filter(|text| !text.is_empty())
        .cloned();
    if work.find(properties, W, "showingPlcHdr").is_none() {
        let shown = make(work, "showingPlcHdr", &[]);
        insert_in_order(work, properties, shown, orders);
    }
    let run = make(work, "r", &[]);
    if let Some(formatting) = work.find(properties, W, "rPr") {
        let copy = work.deep_copy(formatting);
        work.append(run, copy);
    }
    let text = make(work, "t", &[]);
    crate::text::write::set_spaced_text(work, text, named.as_deref().unwrap_or(SPACES));
    work.append(run, text);
    work.append(host, run);
}

/// An element of another vocabulary (an equation, a choice of content): shown.
fn foreign(work: &Work, node: Id) -> bool {
    work.is_element(node) && work.name(node).is_some_and(|n| n.namespace() != Some(W))
}
