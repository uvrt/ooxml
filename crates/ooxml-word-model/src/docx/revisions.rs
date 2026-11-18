//! Owns: a document's revision records as docx-agent lists them (`edit/annotations.py`,
//! `_revision_elements`): every `w:ins`, `w:del`, move, property change and cell change
//! with a `w:id`, in story and document order (`mc:Fallback` content left out), as
//! `rev:<id>` (`#<k>` for a repeat).
//!
//! Must not: group records into changes, accept or reject.

use crate::docx::Docx;
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
        for found in part.facts().revisions() {
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

/// A record as docx-agent's `Revision` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordInfo {
    /// Its kind (`insertion`, `paragraph-mark-deletion`, `row-insertion`, `cell-merge`...).
    pub kind: String,
    /// The id of the paragraph it is in (none for a row's or table's own record).
    pub paragraph: Option<String>,
    /// The text it inserts, deletes or moves (empty for the others).
    pub text: String,
    /// `w:date`.
    pub date: Option<String>,
}

/// `record` described: its kind, paragraph and text.
#[must_use]
pub fn describe_record(docx: &Docx, record: &Record) -> RecordInfo {
    let part = docx.part(&record.part);
    let root = part.map(|p| p.root());
    let above = |up: usize| -> Option<&Element> {
        let n = record.path.len().checked_sub(up)?;
        crate::docx::at_path(root?, record.path.get(..n)?).map(|e| &**e)
    };
    let local = record.element.name().local();
    let base = KINDS.iter().find(|(l, _)| *l == local).map_or("", |(_, k)| *k);
    let is = |e: Option<&Element>, name: &str| e.is_some_and(|e| e.name().is(Some(W), name));
    let kind = if ["ins", "del", "moveFrom", "moveTo"].contains(&local) {
        if is(above(1), "rPr") && is(above(2), "pPr") {
            format!("paragraph-mark-{base}")
        } else if is(above(1), "rPr") {
            format!("run-mark-{base}")
        } else if is(above(1), "trPr") {
            format!("row-{base}")
        } else {
            base.to_owned()
        }
    } else {
        base.to_owned()
    };
    let paragraph = (1..=record.path.len())
        .find(|up| is(above(*up), "p"))
        .and_then(|up| record.path.get(..record.path.len() - up))
        .and_then(|path| part.and_then(|p| p.ids.paragraph_at(path)))
        .map(|p| p.id);
    let text = if ["ins", "del", "moveFrom", "moveTo"].contains(&local) {
        let mut out = String::new();
        texts_below(&record.element, &mut out);
        out
    } else {
        String::new()
    };
    RecordInfo {
        kind,
        paragraph,
        text,
        date: record.element.attribute(Some(W), "date").map(str::to_owned),
    }
}

/// The leading text of every `w:t` and `w:delText` below `element`, in document order.
fn texts_below(element: &Element, out: &mut String) {
    for child in element.child_elements() {
        if child.name().is(Some(W), "t") || child.name().is(Some(W), "delText") {
            out.push_str(&crate::docx::text::leading_text(child));
        }
        texts_below(child, out);
    }
}
