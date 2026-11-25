//! Owns: `check`'s `row_text_format` (docx-agent's `layout_facts.row_text_format`, round 58,
//! R126): of the main story's tables, the rows the session added or wrote into -- named in
//! the session's record of the rows written, by `w14:paraId` -- whose text looks unlike the
//! rows around them ([`crate::rows_unlike`] says which and why).
//!
//! Must not: change the document, or keep the record (it is the session's, handed in): a
//! row the record names that this version does not hold is simply not found.

use crate::names::W14;
use crate::{rows_unlike, EditError, Result, RowUnlike, WordDoc};
use ooxml_word_model::docx::at_path;
use ooxml_word_model::ids::TableId;
use std::collections::BTreeSet;

/// `check`'s `row_text_format` of `doc`: the rows `written` names (`w14:paraId`s) unlike
/// the rows around them, table by table in document order. `keep`: only the tables whose
/// id it keeps are read.
///
/// # Errors
/// When the version cannot be read.
pub fn row_text_format(
    doc: &WordDoc,
    written: &BTreeSet<String>,
    keep: Option<&dyn Fn(&str) -> bool>,
) -> Result<Vec<RowUnlike>> {
    if written.is_empty() {
        return Ok(Vec::new());
    }
    let docx = doc.docx()?;
    let body = docx.body().ok_or_else(|| EditError::no_part(docx.main.clone()))?;
    let mut found = Vec::new();
    let tables = body.ids.tables();
    // Kept ones only: a table's id is named without its rows, those of a table kept named.
    let kept: Box<dyn Iterator<Item = TableId>> = match keep {
        None => Box::new(tables.iter()),
        Some(keep) => Box::new(
            tables
                .placed()
                .filter(|t| tables.head(t.ordinal).is_some_and(|h| keep(&h.id)))
                .filter_map(|t| tables.get(t.ordinal)),
        ),
    };
    for table in kept {
        let marked: Vec<bool> = table
            .rows
            .iter()
            .map(|(path, _)| {
                at_path(body.root(), path)
                    .and_then(|row| row.attribute(Some(W14), "paraId"))
                    .is_some_and(|id| written.contains(id))
            })
            .collect();
        if marked.iter().any(|m| *m) {
            found.extend(rows_unlike(&docx, &body.part, &table, &marked));
        }
    }
    Ok(found)
}
