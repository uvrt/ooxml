//! Owns: the tables' and charts' records a version keeps, as docx-agent's
//! `ProvenanceOps.provenance` reads them: each key whose table or chart the version has
//! with its tracked changes accepted, with its record; the entries left out counted.
//!
//! Must not: read a paragraph's text facts (their entries are skipped, not counted).

use super::{entries, entry_text, item, kind_of, record, Kind};
use crate::address::resolve::direct;
use crate::docx::drawings::graphics;
use crate::docx::{at_path, Docx};
use crate::ids::{Entry, W};
use ooxml_opc::OpcError;
use ooxml_xml::Element;
use serde_json::{Map, Value};

/// The records a version keeps: `(key, record)` in the item's order, and how many entries
/// were left out (their object is gone with the tracked changes accepted, their key is no
/// durable id, their text is no record, or a key repeated).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Provenance {
    /// The records, in order.
    pub records: Vec<(String, Map<String, Value>)>,
    /// The entries left out.
    pub ignored: usize,
}

impl Provenance {
    /// The record kept for `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Map<String, Value>> {
        self.records.iter().find(|(k, _)| k == key).map(|(_, r)| r)
    }

    /// The record kept for `key`, written as docx-agent writes a record.
    #[must_use]
    pub fn text(&self, key: &str) -> Option<String> {
        self.get(key).map(|r| super::dumps(&Value::Object(r.clone())))
    }
}

/// The records `docx` keeps (docx-agent's `provenance()`).
///
/// # Errors
/// When the main part's relationships are not XML.
pub fn provenance(docx: &Docx) -> Result<Provenance, OpcError> {
    let mut out = Provenance::default();
    let Some(name) = item(docx)? else {
        return Ok(out);
    };
    let Some(tree) = docx.package.tree(&name)? else {
        return Ok(out);
    };
    for entry in entries(tree.root()) {
        if entry.attribute(None, "kind") == Some("text") {
            continue;
        }
        let key = entry.attribute(None, "key").unwrap_or_default();
        let Some(found) = record(&entry_text(entry)) else {
            out.ignored += 1;
            continue;
        };
        if !accepted_target(docx, key) || out.get(key).is_some() {
            out.ignored += 1;
            continue;
        }
        out.records.push((key.to_owned(), found));
    }
    Ok(out)
}

/// Whether the table or chart `key` is in `docx` with its tracked changes accepted
/// (docx-agent's `_provenance_target` with `accepted=True`, for a table or a chart).
fn accepted_target(docx: &Docx, key: &str) -> bool {
    match kind_of(key) {
        Some(Kind::Table) => {
            let Some((part, Entry::Table(k))) = direct(docx, key) else {
                return false;
            };
            part.ids
                .tables()
                .get(k)
                .is_some_and(|table| !table.rows.iter().all(|(path, _)| row_deleted(part.root(), path)))
        }
        Some(Kind::Chart) => graphics(docx).into_iter().find(|d| d.id == key).is_some_and(|d| {
            d.kind == "chart"
                && !docx
                    .part(&d.part)
                    .is_some_and(|part| inside_deletion(part.root(), &d.path))
        }),
        Some(Kind::Text) | None => false,
    }
}

/// Whether the row at `path` is deleted as a tracked change (`w:trPr/w:del`).
fn row_deleted(root: &std::sync::Arc<Element>, path: &[u32]) -> bool {
    at_path(root, path)
        .and_then(|row| row.find(Some(W), "trPr"))
        .is_some_and(|properties| properties.find(Some(W), "del").is_some())
}

/// Whether an element above the one at `path` is a tracked deletion or move away (`w:del`,
/// `w:moveFrom`): what a drawing inside is gone with once the changes are accepted.
fn inside_deletion(root: &std::sync::Arc<Element>, path: &[u32]) -> bool {
    (1..path.len()).any(|depth| {
        path.get(..depth)
            .and_then(|above| at_path(root, above))
            .is_some_and(|e| e.name().is(Some(W), "del") || e.name().is(Some(W), "moveFrom"))
    })
}
