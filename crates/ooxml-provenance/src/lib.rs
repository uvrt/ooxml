//! Records of where placed values came from, as every format keeps them (docx-agent's and
//! ooxml-edit's `provenance.py`: Word's text, table and chart records; later a deck's and a
//! workbook's).
//!
//! Owns: a record's text -- written as `ooxml_edit.provenance.dumps` writes it ([`dumps`])
//! and read as `loads` reads it ([`record`]).
//!
//! Must not: know where a format keeps its records, or what a record says.
#![forbid(unsafe_code)]

mod json;

pub use json::dumps;

use serde_json::{Map, Value};

/// The record a text holds (`ooxml_edit.provenance.loads`): a JSON object with a whole
/// number `v` of at least 1; `None` for anything else.
#[must_use]
pub fn record(text: &str) -> Option<Map<String, Value>> {
    let Ok(Value::Object(found)) = serde_json::from_str::<Value>(text) else {
        return None;
    };
    found.get("v").and_then(Value::as_u64).filter(|v| *v >= 1)?;
    Some(found)
}
