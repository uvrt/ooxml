//! Owns: a document's revision records as docx-agent lists them (`edit/annotations.py`,
//! `_revision_elements`): every `w:ins`, `w:del`, move, property change and cell change
//! with a `w:id`, in story and document order (`mc:Fallback` content left out), as
//! `rev:<id>` (`#<k>` for a repeat).
//!
//! Must not: group records into changes, accept or reject.

use crate::docx::{live_paths, Docx};
use crate::ids::W;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// The revision kinds, by local name in `w:`.
pub const KINDS: [(&str, &str); 15] = [
    ("ins", "insertion"),
    ("del", "deletion"),
    ("moveFrom", "move-from"),
    ("moveTo", "move-to"),
    ("rPrChange", "run-properties"),
    ("pPrChange", "paragraph-properties"),
    ("sectPrChange", "section-properties"),
    ("tblPrChange", "table-properties"),
    ("trPrChange", "row-properties"),
    ("tcPrChange", "cell-properties"),
    ("tblGridChange", "table-grid"),
    ("numberingChange", "numbering"),
    ("cellIns", "cell-insertion"),
    ("cellDel", "cell-deletion"),
    ("cellMerge", "cell-merge"),
];

/// A revision record.
#[derive(Debug, Clone)]
pub struct Record {
    /// Its part.
    pub part: String,
    /// Its id.
    pub id: String,
    /// The element.
    pub element: Arc<Element>,
    /// Its place in the part.
    pub path: Box<[u32]>,
}

impl Record {
    /// Its author (`w:author`).
    #[must_use]
    pub fn author(&self) -> Option<&str> {
        self.element.attribute(Some(W), "author")
    }
}

/// Whether `element` is a revision record's element.
#[must_use]
pub fn is_revision(element: &Element) -> bool {
    element.name().namespace() == Some(W) && KINDS.iter().any(|(local, _)| element.name().local() == *local)
}

/// Every revision record, in story and document order.
#[must_use]
pub fn records(docx: &Docx) -> Vec<Record> {
    let mut out = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for part in &docx.parts {
        for found in live_paths(part.root(), &[], &is_revision) {
            let Some(raw) = found.element.attribute(Some(W), "id") else {
                continue;
            };
            let count = seen.entry(raw.to_owned()).or_insert(0);
            let id = if *count > 0 {
                format!("rev:{raw}#{count}")
            } else {
                format!("rev:{raw}")
            };
            *count += 1;
            out.push(Record {
                part: part.part.clone(),
                id,
                element: found.element,
                path: found.path,
            });
        }
    }
    out
}
