//! Owns: `word_edit_table`'s refusals of a tracked reshape that would move a pending
//! tracked merge to another cell (docx-agent's `tools/objects.py`,
//! `_no_reshape_over_merge_lead`, the tracked half of `_no_split_over_pending` and
//! `_settle_first`, round 55, R115): a row or column deleted that holds a merge's first
//! cell, a split of a cell a merge makes -- refused naming the changes that settle the
//! merge, by id, with the call that accepts them (rule 2).
//!
//! Must not: change the document.

use super::table_view::{AcceptedView, TableView};
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{Refusal, ToolError, WayOut};
use ooxml_word_edit::{merges_in_the_way, Reshape};
use ooxml_word_model::docx::changes::changes_holding;
use ooxml_word_model::docx::Docx;
use serde_json::json;

/// Tracked, `reshape` refused where it reaches a pending tracked merge it would move.
pub(super) fn no_reshape_over_merge(
    doc: &str,
    docx: &Docx,
    view: &TableView,
    accepted: &AcceptedView,
    reshape: Reshape,
) -> Result<(), ToolError> {
    let Some(records) = merges_in_the_way(&view.table.element, reshape) else {
        return Ok(());
    };
    let table = &view.table.id;
    let shape = &accepted.table;
    let message = match reshape {
        Reshape::DeleteRow(index) => format!(
            "row {} of {table} holds the first cell of a pending tracked merge; deleting it would move the merge \
             to another cell",
            shape.from_row(index)
        ),
        Reshape::DeleteColumn(index) => format!(
            "column {} of {table} holds the first cell of a pending tracked merge; deleting it would move the \
             merge to another cell",
            shape.from_grid(index)
        ),
        Reshape::Split(row, column) => format!(
            "({}, {}) of {table} is a cell a pending tracked merge makes; a split of it is written once the merge \
             is settled",
            shape.from_row(row),
            shape.from_grid(column)
        ),
    };
    let wanted: Vec<(String, Vec<u32>)> = records
        .into_iter()
        .map(|below| {
            (
                view.part.clone(),
                view.table.path.iter().copied().chain(below).collect(),
            )
        })
        .collect();
    Err(settle_first(doc, table, &changes_holding(docx, &wanted), &message))
}

/// A refusal whose way out is accepting the pending changes `ids` in the way -- only those,
/// by id (`_settle_first`, rule 2).
fn settle_first(doc: &str, table: &str, ids: &[String], message: &str) -> ToolError {
    let listed = format!("[{}]", ids.iter().map(|i| repr_str(i)).collect::<Vec<_>>().join(", "));
    let message = format!(
        "{message}. Accept that change first (word_changes accept ids {listed}; reject keeps the cells apart \
         instead), read {table} again, then retry"
    );
    let options = [
        format!("word_changes accept ids {listed}"),
        format!("word_changes reject ids {listed}"),
    ];
    let way_out = WayOut::names(options.clone()).unwrap_or_else(|| WayOut::one(json!(options[0])));
    ToolError::refusal(Refusal::Refused, message, way_out)
        .with_detail(
            "fix",
            json!({"tool": "word_changes", "calls": [{"action": "accept", "ids": ids}]}),
        )
        .with_detail("ids", json!(ids))
        .with_detail(
            "call",
            json!({"tool": "word_changes", "arguments": {"doc": doc, "action": "accept", "ids": ids}}),
        )
}
