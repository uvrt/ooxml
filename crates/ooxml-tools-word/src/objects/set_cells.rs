//! Owns: `word_edit_table`'s `set_cells` without a source (docx-agent's `tools/objects.py`,
//! `_edit_table`): the cells read as places ([`super::cell_fills`]), at most 5,000 in a
//! call, each entry's text (`_cell_value`), different values for one merged cell refused
//! with the split that gives each its cell (`_one_value_per_cell`, rule 2), the cells
//! written as one command (labels read as the earlier cells left them), and the reply: the
//! table as `table_json` gives it, a fill said by its range and counts (rule 4) with the
//! table's paragraphs named by the table.
//!
//! Positions and labels count the table as accepting its tracked changes leaves it
//! (docx-agent's `_Accepted`): the edit core resolves them as the earlier cells left the
//! table, and the notes they make are said in the reply.
//!
//! The host's, said in `unported`: a value by reference (a cell's or the call's `from`, a
//! `{{...}}` in a text), a table placed from a source.
//!
//! Must not: change the document but through the command.

use super::cell_fills::{column_rows, count_fills, fill_brief, Fill};
use super::edit_table::{table_json, unported};
use super::edit_table_args::{CellArg, WordEditTable};
use super::table_view::{table_view, AcceptedView, TableView};
use crate::describe::short;
use crate::review::{needed, Editing};
use crate::structure::common::{outcome, policy, refused, Outcome};
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{Failure, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{CellAxis, CellEdit, CellWrite, EditError, SetCells, WordDoc};
use ooxml_word_model::docx::tables::{Axis, TableGrid};
use ooxml_word_model::docx::Docx;
use serde_json::{json, Value};
use std::collections::HashSet;

/// Cells one call may write.
const MOST_CELLS: usize = 5000;
/// Merged cells a refusal's details list (the rest counted).
const LISTED: usize = 20;

impl From<CellEdit> for Outcome {
    fn from(edit: CellEdit) -> Self {
        Outcome {
            id: edit.id,
            changed: true,
            created: edit.created,
            removed: edit.removed,
            renamed: edit.renamed,
            warnings: Vec::new(),
        }
    }
}

/// `set_cells` on the table `view` (found on `docx`, the version before).
pub(super) fn set_cells(
    args: &WordEditTable,
    editing: &Editing<'_>,
    docx: &Docx,
    view: &TableView,
    accepted: &mut AcceptedView,
    retry: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    let cells = args
        .cells
        .as_deref()
        .ok_or_else(|| needed("cells", "", retry.clone()))?;
    let by_reference = |cell: &CellArg| {
        cell.from.as_deref().is_some_and(|f| !f.is_empty()) || cell.text.as_deref().is_some_and(|t| t.contains("{{"))
    };
    if cells.iter().any(by_reference) {
        return Ok(unported(
            "word_edit_table set_cells values by reference (from, {{...}})",
            retry,
        ));
    }
    let part = docx
        .part(&view.part)
        .ok_or_else(|| ToolError::internal(format!("no part {}", view.part), "word:edit_table"))?;
    let grid = TableGrid::new(part, &view.table);
    let (places, owners, mut fills) = column_rows(view, &grid, accepted, cells, retry)?;
    let held = if fills.is_empty() {
        HashSet::new()
    } else {
        paragraphs_of(docx, view)
    };
    if places.len() > MOST_CELLS {
        return Err(ToolError::failure(
            Failure::Limit,
            format!("{} cells in one call; at most {MOST_CELLS}", places.len()),
        ));
    }
    let values = cells
        .iter()
        .enumerate()
        .map(|(k, cell)| cell_value(cell, k, retry))
        .collect::<Result<Vec<String>, ToolError>>()?;
    let texts: Vec<&str> = owners
        .iter()
        .map(|k| values.get(*k).map(String::as_str).unwrap_or_default())
        .collect();
    one_value_per_cell(view, &grid, accepted, &places, &owners, &texts)?;
    let writes = places
        .iter()
        .zip(&texts)
        .map(|(place, text)| CellWrite {
            row: axis(place.row, place.row_label.as_deref()),
            column: axis(place.col, place.col_label.as_deref()),
            text: (*text).to_owned(),
        })
        .collect();
    let command = SetCells {
        table: args.table.clone(),
        writes,
        stamping: true,
        policy: policy(editing),
        accepted: (accepted.table.active() || accepted.table.rows_active())
            .then(|| std::sync::Arc::new(accepted.table.clone())),
    };
    // round 58 (R126): the rows written looked at against the rows around them, in the
    // call's transaction
    let look = |_: &_, doc: &WordDoc| Some((doc.ids().follow(&view.table.id), None));
    let (done, rows) = match super::rows_written::run_writing_rows(editing, &command, &view.table.id, look, retry) {
        Ok(done) => done,
        Err(error @ (EditError::Address(_) | EditError::Argument { .. } | EditError::Position(_))) => {
            return Ok(ToolResult::failed(crate::review::edit_refusal(
                error,
                "",
                retry.clone(),
            )));
        }
        Err(error) => return Ok(refused(error, retry.clone())),
    };
    count_fills(&mut fills, &values);
    for note in done.notes {
        if !accepted.notes.contains(&note) {
            accepted.notes.push(note);
        }
    }
    let notes = std::mem::take(&mut accepted.notes);
    let mut result = reply(editing, view, done.edits, (&fills, notes), &held, retry);
    result.warnings.extend(rows);
    Ok(result)
}

/// A cells entry's row or column: its place, else its label, else missing.
fn axis(at: Option<i64>, label: Option<&str>) -> CellAxis {
    match (at, label) {
        (Some(at), _) => CellAxis::At(usize::try_from(at).unwrap_or(usize::MAX)),
        (None, Some(label)) => CellAxis::Label(label.to_owned()),
        (None, None) => CellAxis::Missing,
    }
}

/// A cells entry's text (`_cell_value` without a `from`).
fn cell_value(cell: &CellArg, k: usize, retry: &TryCall) -> Result<String, ToolError> {
    cell.text.clone().ok_or_else(|| {
        ToolError::refusal(
            Refusal::InvalidArguments,
            format!("cells[{k}] gives no text: give text, or from (one value, d:<id>/#12/<column>)"),
            WayOut::names(["text", "from"]).unwrap_or_else(|| WayOut::call(retry.clone())),
        )
        .at(format!("cells[{k}].text"))
    })
}

/// The ids of the table's paragraphs, those of tables nested in it too (`_paragraphs_of`).
fn paragraphs_of(docx: &Docx, view: &TableView) -> HashSet<String> {
    docx.part(&view.part)
        .map(|part| {
            part.ids
                .paragraphs_within(&view.table.path)
                .into_iter()
                .map(|p| p.id)
                .collect()
        })
        .unwrap_or_default()
}

/// A cells entry's grid place (document row, grid column) as the table is accepted, where
/// it has one (`_locate`, its refusals left for the write; its notes made).
fn place_of(grid: &TableGrid<'_>, accepted: &mut AcceptedView, cell: &CellArg) -> Option<(usize, usize)> {
    let row = match (cell.row, cell.row_label.as_deref()) {
        (Some(row), _) => Axis::At(row),
        (None, Some(label)) => Axis::Label(label),
        (None, None) => return None,
    };
    let column = match (cell.col, cell.col_label.as_deref()) {
        (Some(col), _) => Axis::At(col),
        (None, Some(label)) => Axis::Label(label),
        (None, None) => return None,
    };
    accepted.table.locate(grid, (row, column), &mut accepted.notes).ok()
}

/// One origin's writes: (entry, position given, text).
type Hits<'t> = Vec<(usize, (usize, usize), &'t str)>;

/// A merged cell given different values.
struct Clash<'t> {
    origin: (usize, usize),
    hits: Hits<'t>,
    /// Its grid's rows and columns.
    size: (usize, usize),
    /// The first position given.
    first: (usize, usize),
}

/// Different values given to positions of one merged cell, refused (`_one_value_per_cell`,
/// round 57, R125): it holds one text, so all but the last would be lost. Each such cell is
/// named with its positions and values, and the split that gives each value a cell.
fn one_value_per_cell(
    view: &TableView,
    grid: &TableGrid<'_>,
    accepted: &mut AcceptedView,
    places: &[CellArg],
    owners: &[usize],
    texts: &[&str],
) -> Result<(), ToolError> {
    let mut found: Vec<((usize, usize), Hits<'_>)> = Vec::new();
    for ((cell, owner), text) in places.iter().zip(owners).zip(texts) {
        let Some((row, col)) = place_of(grid, accepted, cell) else {
            continue;
        };
        let Some(origin) = view.origin(row, col) else { continue };
        let given = match (cell.row, cell.col) {
            (Some(r), Some(c)) => (usize::try_from(r).unwrap_or(row), usize::try_from(c).unwrap_or(col)),
            _ => (row, col),
        };
        match found.iter_mut().find(|(o, _)| *o == origin) {
            Some((_, hits)) => hits.push((*owner, given, text)),
            None => found.push((origin, vec![(*owner, given, text)])),
        }
    }
    let clashes: Vec<Clash<'_>> = found
        .into_iter()
        .filter(|(_, hits)| {
            hits.iter().map(|h| h.1).collect::<HashSet<_>>().len() > 1
                && hits.iter().map(|h| h.2).collect::<HashSet<_>>().len() > 1
        })
        .map(|(origin, hits)| {
            let covered: Vec<(usize, usize)> = view
                .grid
                .iter()
                .enumerate()
                .flat_map(|(r, line)| {
                    line.iter()
                        .enumerate()
                        .filter(|(_, o)| **o == Some(origin))
                        .map(move |(c, _)| (r, c))
                })
                .collect();
            let rows = covered.iter().map(|p| p.0).collect::<HashSet<_>>().len();
            let columns = covered.iter().map(|p| p.1).collect::<HashSet<_>>().len();
            let first = hits.iter().map(|h| h.1).min().unwrap_or(origin);
            Clash {
                origin,
                hits,
                size: (rows, columns),
                first,
            }
        })
        .collect();
    match clashes.first() {
        None => Ok(()),
        Some(first) => Err(clash_refusal(view, first, &clashes)),
    }
}

/// The refusal naming the first merged cell given different values; every one in details.
fn clash_refusal(view: &TableView, first: &Clash<'_>, clashes: &[Clash<'_>]) -> ToolError {
    let id = &view.table.id;
    let named: Vec<String> = first.hits.iter().map(|h| format!("({}, {})", h.1 .0, h.1 .1)).collect();
    let values: Vec<String> = first.hits.iter().map(|h| repr_str(&short(h.2, 40))).collect();
    let more = if clashes.len() > 1 {
        format!(" ({} more merged cell(s) in details)", clashes.len() - 1)
    } else {
        String::new()
    };
    let message = format!(
        "{} are one merged cell of {id}: it holds one text, so of {} only the last would stay{more}; nothing \
         written. Give each value a cell first: split ({}, {}) into {} row(s) x {} column(s) (the cell's own \
         grid), or insert_row for a new row with a cell per column; or give the cell one value",
        named.join(", "),
        values.join(", "),
        first.first.0,
        first.first.1,
        first.size.0,
        first.size.1,
    );
    let listed: Vec<Value> = clashes
        .iter()
        .take(LISTED)
        .map(|c| {
            json!({
                "cell": format!("{id}/c{},{}", c.origin.0, c.origin.1),
                "positions": c.hits.iter().map(|h| json!([h.1 .0, h.1 .1])).collect::<Vec<_>>(),
                "values": c.hits.iter().map(|h| short(h.2, 40)).collect::<Vec<_>>(),
                "cells": c.hits.iter().map(|h| h.0).collect::<Vec<_>>(),
                "split": {"tool": "word_edit_table", "arguments": {"table": id, "action": "split",
                    "first": {"row": c.first.0, "col": c.first.1}, "rows": c.size.0, "columns": c.size.1}},
            })
        })
        .collect();
    let error = ToolError::refusal(
        Refusal::Refused,
        message,
        WayOut::names(["split", "insert_row"]).unwrap_or_else(|| WayOut::one(json!("split"))),
    )
    .at("cells")
    .with_detail("merged", json!(listed));
    match clashes.len().saturating_sub(LISTED) {
        0 => error,
        left_out => error.with_detail("merged_left_out", json!(left_out)),
    }
}

/// The reply as `_edit_table` gives it after `set_cells`.
fn reply(
    editing: &Editing<'_>,
    view: &TableView,
    edits: Vec<CellEdit>,
    (fills, notes): (&[Fill], Vec<String>),
    held: &HashSet<String>,
    retry: &TryCall,
) -> ToolResult<Value> {
    let table = view.table.id.clone();
    let head = editing.editor.store().snapshot();
    let now = head.doc().ids().follow(&table);
    let docx = head.doc().docx().ok();
    let mut data = docx
        .as_ref()
        .and_then(|docx| table_json(docx, &now, head.doc(), fills.is_empty(), retry));
    if let Some(Value::Object(map)) = &mut data {
        if !fills.is_empty() {
            map.insert("filled".into(), json!(fills.iter().map(fill_brief).collect::<Vec<_>>()));
        }
    }
    let edits: Vec<Outcome> = edits.into_iter().map(Outcome::from).collect();
    let mut result = outcome(&edits, format!("set_cells on {table}"), data);
    if let ooxml_tools::Outcome::Done(found) = &mut result.outcome {
        if !found.changed.contains(&now) {
            found.changed.push(now.clone());
        }
        if !fills.is_empty() {
            // the cells' paragraphs are the table's: named by it, not one by one (rule 4)
            let mut inside = held.clone();
            if let Some(after) = docx.as_ref().and_then(|d| {
                table_view(head.doc(), d, &now, retry)
                    .ok()
                    .map(|v| paragraphs_of(d, &v))
            }) {
                inside.extend(after);
            }
            let cells = format!("{now}/");
            let keep = |ids: &mut Vec<String>| {
                ids.retain(|i| !inside.contains(i) && !i.starts_with(&cells) && *i != now);
            };
            keep(&mut found.changed);
            found.changed.insert(0, now.clone());
            keep(&mut found.created);
            keep(&mut found.removed);
        }
        if now != table {
            match found.renamed.iter_mut().find(|(old, _)| *old == table) {
                Some(entry) => entry.1.clone_from(&now),
                None => found.renamed.push((table, now)),
            }
        }
    }
    result.warnings.extend(notes);
    for fill in fills {
        result.warnings.extend(fill.notes.iter().cloned());
    }
    result
}
