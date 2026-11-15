//! What a fields update reads of one laid-out state (docx-agent's `word_fields` update):
//! the first page each asked paragraph is drawn on, and every page's number, section,
//! story kind and blankness -- docx2svg's `DocumentLayout.where(...)[0].page` and
//! `pages[k].info`, from a snapshot.  The update computes its results (page numbers in the
//! section's format, contents rebuilt) in Python from these.

use crate::engine::Snap;
use serde_json::{json, Value};

/// `{"version", "pages", "first": [1-based page or null per key], "info": [[number,
/// section, story, blank] per page] (only where `info`)}`.  A paragraph's first page is
/// the page of its first line (a cell paragraph's: the first page any of its lines is
/// drawn on), as `Snap::pages_of` finds them.
pub fn field_pages(s: &Snap, keys: &[String], info: bool) -> Value {
    let first: Vec<Value> = s
        .pages_of(keys)
        .iter()
        .map(|p| p.first().map_or(Value::Null, |x| json!(x)))
        .collect();
    let mut out = json!({"version": s.version, "pages": s.page_count(), "first": first});
    if info {
        out["info"] = Value::Array(
            s.pag
                .info
                .iter()
                .map(|i| json!([i.number, i.section, i.story, i.blank]))
                .collect(),
        );
    }
    out
}
