//! Owns: the document-wide lookups a reading starts from: every note with its id
//! (`fn:1`, `en:2`), by its reference's kind and `w:id`, and the bookmark names internal
//! hyperlinks point at (shown even when hidden, as `_Toc` ones are).
//!
//! Must not: read a story's blocks.

use super::{index, Placed};
use crate::docx::Docx;
use crate::ids::W;
use ooxml_xml::Node;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The notes by `(fn|en, w:id)`.
pub(super) fn notes_of(docx: &Docx) -> HashMap<(String, String), String> {
    let mut out = HashMap::new();
    for (part, identifier, _) in note_elements(docx) {
        let _ = part;
        if let Some((prefix, id)) = identifier.split_once(':') {
            out.insert((prefix.to_owned(), id.to_owned()), identifier.clone());
        }
    }
    out
}

/// Every footnote and endnote (separators aside): its part, id (`fn:1`) and element.
#[must_use]
pub fn note_elements(docx: &Docx) -> Vec<(String, String, Placed)> {
    let mut out = Vec::new();
    for part in &docx.parts {
        let root = part.root();
        let prefix = if root.name().is(Some(W), "footnotes") {
            "fn"
        } else if root.name().is(Some(W), "endnotes") {
            "en"
        } else {
            continue;
        };
        for (k, node) in root.children().iter().enumerate() {
            let Node::Element(note) = node else { continue };
            let is_note = note.name().is(Some(W), "footnote") || note.name().is(Some(W), "endnote");
            if is_note && matches!(note.attribute(Some(W), "type"), None | Some("normal")) {
                let id = note.attribute(Some(W), "id").unwrap_or("None");
                out.push((
                    part.part.clone(),
                    format!("{prefix}:{id}"),
                    Placed {
                        element: Arc::clone(note),
                        path: vec![index(k)].into_boxed_slice(),
                    },
                ));
            }
        }
    }
    out
}

/// Bookmark names an internal hyperlink points at.
pub(super) fn anchors(docx: &Docx) -> HashSet<String> {
    let mut out = HashSet::new();
    for part in &docx.parts {
        let mut stack = vec![Arc::clone(part.root())];
        while let Some(e) = stack.pop() {
            if e.name().is(Some(W), "hyperlink") {
                if let Some(a) = e.attribute(Some(W), "anchor").filter(|a| !a.is_empty()) {
                    out.insert(a.to_owned());
                }
            }
            stack.extend(e.child_elements().cloned());
        }
    }
    out
}
