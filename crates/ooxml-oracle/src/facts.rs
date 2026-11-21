//! `word-reads`' fact cases: `check`'s facts read from the document alone
//! (`ooxml_word_edit::facts`, worded by `ooxml_tools_word`), in the form
//! `python/word_facts_cases.py` writes docx-agent's (`layout_facts.py`'s functions):
//! `{"facts": [...]}`, or `{"error"}`.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::Unlimited;
use ooxml_opc::Package;
use ooxml_tools_word::{empty_run_json, grid_mismatch_json, lost_cell_json, skipped_level_json};
use ooxml_word_edit::{facts, EditError, WordDoc};
use serde_json::{json, Value};
use std::sync::Arc;

/// The fact `fact` of `package`'s first version, uncapped.
#[must_use]
pub fn reply(package: &Package, fact: &str) -> Value {
    match WordDoc::open(package.clone(), Arc::new(Unlimited)).and_then(|doc| entries(&doc, fact)) {
        Ok(Some(found)) => json!({"facts": found}),
        Ok(None) => json!({"error": format!("no fact {fact}")}),
        Err(error) => json!({"error": error.to_string()}),
    }
}

fn entries(doc: &WordDoc, fact: &str) -> Result<Option<Vec<Value>>, EditError> {
    Ok(Some(match fact {
        "empty_paragraphs" => facts::empty_paragraphs(doc)?.iter().map(empty_run_json).collect(),
        "numbering_skips_level" => facts::numbering_skips_level(doc)?
            .iter()
            .map(skipped_level_json)
            .collect(),
        "table_grid_mismatch" => facts::table_grid_mismatch(doc, None)?
            .iter()
            .map(grid_mismatch_json)
            .collect(),
        "accept_loses_text" => facts::accept_loses_text(doc)?.iter().map(lost_cell_json).collect(),
        _ => return Ok(None),
    }))
}
