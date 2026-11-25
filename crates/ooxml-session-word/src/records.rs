//! Owns: a Word version's records of placed values as its host reads them
//! ([`Records::records`]): the tables' and charts' records (docx-agent's `provenance()`,
//! with the entries left out counted), and the paragraphs' text facts and labels
//! (`text_facts()`, `text_labels()`), read from the version's custom XML item
//! (`ooxml_word_model::docx::provenance`) with the version's renames.
//!
//! Must not: change the records (every edit's follow-up does, `ooxml_word_edit`).

use crate::WordVersion;
use ooxml_session::{Opened, SessionError};
use ooxml_word_model::docx::provenance::{provenance, text_records, ByParagraph};
use serde_json::{Map, Value};

/// A Word version's records of placed values.
pub trait Records {
    /// The records this version keeps, as JSON: `{"provenance": [[key, record], ...],
    /// "ignored": n, "facts": [[paragraph, [fact, ...]], ...], "labels": [...]}`, each list
    /// in the item's order; `None` for a document that is not a Word document.
    ///
    /// # Errors
    /// When a story part or the main part's relationships are not XML.
    fn records(&self) -> Result<Option<Value>, SessionError>;
}

impl Records for Opened {
    fn records(&self) -> Result<Option<Value>, SessionError> {
        let Some(word) = self.word() else {
            return Ok(None);
        };
        let docx = word.docx().map_err(SessionError::format)?;
        let kept = provenance(&docx)?;
        let text = text_records(&docx, word.ids().aliases())?;
        let mut out = Map::new();
        let records = kept
            .records
            .into_iter()
            .map(|(key, record)| Value::Array(vec![Value::from(key), Value::Object(record)]))
            .collect();
        out.insert("provenance".into(), Value::Array(records));
        out.insert("ignored".into(), Value::from(kept.ignored));
        out.insert("facts".into(), by_paragraph(text.facts));
        out.insert("labels".into(), by_paragraph(text.labels));
        Ok(Some(Value::Object(out)))
    }
}

fn by_paragraph(found: ByParagraph) -> Value {
    Value::Array(
        found
            .into_iter()
            .map(|(id, list)| Value::Array(vec![Value::from(id), Value::Array(list)]))
            .collect(),
    )
}
