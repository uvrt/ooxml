//! `word-reads`' fact cases: `check`'s facts read from the document alone
//! (`ooxml_word_edit::facts`, worded by `ooxml_tools_word`), in the form
//! `python/word_facts_cases.py` writes docx-agent's (`layout_facts.py`'s functions):
//! `{"facts": [...]}`, or `{"error"}`.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::Unlimited;
use ooxml_opc::Package;
use ooxml_tools_word::{empty_run_json, grid_mismatch_json, lost_cell_json, row_unlike_json, skipped_level_json};
use ooxml_word_edit::{facts, EditError, WordDoc};
use ooxml_word_model::docx::at_path;
use ooxml_word_model::ids::W14;
use serde_json::{json, Value};
use std::collections::BTreeSet;
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
        "row_text_format" => facts::row_text_format(doc, &written(doc)?, None)?
            .iter()
            .map(row_unlike_json)
            .collect(),
        _ => return Ok(None),
    }))
}

/// The made-up rows written `row_text_format` is read with (`word_facts_cases.py`'s
/// `_written`): each body table's every third row from row 2, by `w14:paraId`.
fn written(doc: &WordDoc) -> Result<BTreeSet<String>, EditError> {
    let docx = doc.docx()?;
    let Some(body) = docx.body() else {
        return Ok(BTreeSet::new());
    };
    Ok(body
        .ids
        .tables()
        .iter()
        .flat_map(|t| t.rows.into_iter().skip(2).step_by(3))
        .filter_map(|(path, _)| {
            at_path(body.root(), &path)?
                .attribute(Some(W14), "paraId")
                .map(str::to_owned)
        })
        .collect())
}
