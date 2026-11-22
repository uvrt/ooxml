//! Owns: `word_edit_table`'s `merge` and `split` read into their change (docx-agent's
//! `tools/objects.py`, `_edit_table`): the corners counted as the table is accepted
//! (`_corner`), untracked a merge or split reaching over a row or column whose deletion is
//! pending, or a cell a pending change takes in, refused with the way out
//! (`_no_merge_over_pending`), and a split past a cell's span refused with the split that
//! keeps the grid (`_no_split_past_span`, rule 2).
//!
//! Tracked, a split of a cell a pending tracked merge makes refused naming the merge's
//! changes to settle first (`_no_split_over_pending`, [`super::edit_table_pending`]).
//!
//! Must not: change the document.

use super::edit_table_args::{Position, TableAction, WordEditTable};
use super::edit_table_pending::no_reshape_over_merge;
use super::table_view::{AcceptedView, TableView};
use crate::review::needed;
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use ooxml_word_edit::{Reshape, TableChange};
use ooxml_word_model::docx::Docx;
use serde_json::json;

/// A merge's or split's change.
pub(super) fn cells_change(
    args: &WordEditTable,
    (tracked, docx): (bool, &Docx),
    view: &TableView,
    accepted: &mut AcceptedView,
    retry: &TryCall,
) -> Result<TableChange, ToolError> {
    let count = |n: i64| usize::try_from(n).unwrap_or(usize::MAX);
    if args.action == TableAction::Merge {
        let first = args.first.as_ref().ok_or_else(|| needed("first", "", retry.clone()))?;
        let last = args.last.as_ref().ok_or_else(|| needed("last", "", retry.clone()))?;
        let (r1, c1) = accepted.place(first.row, first.col, "first", retry)?;
        let (r2, c2) = accepted.place(last.row, last.col, "last", retry)?;
        if !tracked {
            no_merge_over_pending(args, accepted, (r1, r2), (c1, c2), "merge")?;
        }
        return Ok(TableChange::Merge {
            first: (r1, c1),
            last: (r2, c2),
        });
    }
    let first = args
        .first
        .as_ref()
        .ok_or_else(|| needed("first", " (the cell to split)", retry.clone()))?;
    let (row, column) = accepted.place(first.row, first.col, "first", retry)?;
    if tracked {
        no_reshape_over_merge(&args.doc, docx, view, accepted, Reshape::Split(row, column))?;
    } else {
        no_merge_over_pending(args, accepted, (row, row), (column, column), "split")?;
    }
    let columns = args.columns.filter(|c| *c != 0).map(count);
    no_split_past_span(args, view, first, (row, column), columns)?;
    Ok(TableChange::Split {
        row,
        column,
        rows: args.rows.map_or(1, count),
        columns,
    })
}

/// Untracked, a merge or split over document rows `rows` and grid columns `columns`
/// reaching over a row or column whose deletion is pending, or a cell a pending change
/// takes in, refused: it would settle that change for some cells only. The way out accepts
/// the table's changes first (`_no_merge_over_pending`, round 54).
fn no_merge_over_pending(
    args: &WordEditTable,
    accepted: &AcceptedView,
    (r1, r2): (usize, usize),
    (c1, c2): (usize, usize),
    action: &str,
) -> Result<(), ToolError> {
    let table = &accepted.table;
    let rows_over = table.rows_active() && (r1..=r2).any(|r| !table.rows.contains(&r));
    if !table.active() && !rows_over {
        return Ok(());
    }
    let over = (c1..=c2).any(|c| !table.kept.contains(&c));
    let taken = table
        .given
        .keys()
        .any(|(r, c)| (r1..=r2).contains(r) && (c1..=c2).contains(c));
    if !over && !taken && !rows_over {
        return Ok(());
    }
    let what = if rows_over {
        "a row deleted"
    } else if over {
        "a column deleted"
    } else {
        "a cell merged"
    };
    let id = &table.table;
    let message = format!(
        "the {action} reaches over {what} by a tracked change not yet accepted in {id}; accept the table's changes \
         first (word_changes accept within {id}), then {action}"
    );
    let fix = json!({"tool": "word_changes", "calls": [{"action": "accept", "within": id}]});
    let call = json!({"tool": "word_changes", "arguments": {"doc": args.doc, "action": "accept", "within": id}});
    let way_out = WayOut::names([format!("word_changes accept within {id}")]).unwrap_or_else(|| WayOut::one(json!(id)));
    Err(ToolError::refusal(Refusal::Refused, message, way_out)
        .with_detail("fix", fix)
        .with_detail("call", call))
}

/// A cell over several grid columns split into more cells than it spans, refused
/// (`_no_split_past_span`, round 57): Word would add grid columns, changing every other
/// row's cells; into 2 up to its span keeps the grid. `(row, column)` is the cell's
/// document row and grid column; `at` the position given.
fn no_split_past_span(
    args: &WordEditTable,
    view: &TableView,
    at: &Position,
    (row, column): (usize, usize),
    columns: Option<usize>,
) -> Result<(), ToolError> {
    let Some(columns) = columns else { return Ok(()) };
    let Some((_, span)) = view
        .row_cells(row)
        .into_iter()
        .find(|(start, span)| *start <= column && column < start + span)
    else {
        return Ok(());
    };
    if span < 2 || columns <= span {
        return Ok(());
    }
    let keeps: Vec<usize> = (2..=span).collect();
    let listed: Vec<String> = keeps.iter().map(ToString::to_string).collect();
    let message = format!(
        "cell ({}, {}) of {} spans {span} grid columns: a split into {columns} would add grid columns, changing \
         every other row's cells (Word's Split Cells does that); into {} keeps the grid ({span}: a cell per column). \
         For {columns}, split into {span}, then split one of the new cells",
        at.row,
        at.col,
        view.table.id,
        listed.join(", ")
    );
    let options = WayOut::options(keeps.iter().map(|k| json!(k)).collect()).unwrap_or_else(|| WayOut::one(json!(span)));
    let call = json!({"tool": "word_edit_table", "arguments": {"doc": args.doc, "table": view.table.id,
        "action": "split", "first": {"row": at.row, "col": at.col}, "columns": span}});
    Err(ToolError::refusal(Refusal::Refused, message, options)
        .at("columns")
        .with_detail("call", call))
}
