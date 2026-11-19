//! Owns: `word_edit_table`'s handler (docx-agent's `tools/objects.py`, `_edit_table`): its
//! arguments read as docx-agent reads them (what belongs to `sync` or to `from` refused
//! elsewhere, rule 2), a row inserted or copied or a column inserted as one command (a
//! row not copied refused with the calls that add one like it), and the reply: the edit's
//! ids, the table's id after it (renamed where a row at the top changed), and the table as
//! `table_json` gives it (its size as accepted and its text as plain Markdown).
//!
//! A row or column deleted says what of the file's own text it took out of the final view
//! ([`crate::originals`]).
//!
//! Cells merged or split, a merge already made answered as changing nothing (rule 3), a
//! split past a cell's span refused with the split that keeps the grid (rule 2).
//!
//! Not ported yet, said in `unported` for the host to answer its own way: `set_cells`,
//! `sync`, `from`; a row holding a drawing or a control copied; a table with pending
//! tracked structure or a placement record.
//!
//! Must not: change the document but through the command.

use super::edit_table_args::{Position, Side, TableAction, WordEditTable};
use super::table_view::{table_view, TableView};
use crate::originals::{add, Watch};
use crate::review::{needed, Editing};
use crate::structure::common::{call, outcome, policy, refused, Outcome};
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{ArgEnum, ErrorCode, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::WordDoc;
use ooxml_word_edit::{EditError, EditTable, TableChange, TableChanged};
use ooxml_word_model::docx::tables::TableGrid;
use ooxml_word_model::docx::{find_path, Docx};
use ooxml_word_model::ids::{TableId, W};
use ooxml_word_model::markdown::to_plain_markdown;
use serde_json::{json, Map, Value};

impl From<TableChanged> for Outcome {
    fn from(edit: TableChanged) -> Self {
        Outcome {
            id: edit.id,
            changed: !edit.unchanged,
            created: edit.created,
            removed: edit.removed,
            renamed: edit.renamed,
            warnings: edit.warnings,
        }
    }
}

/// `word_edit_table` (docx-agent's `word_edit_table`): a table's rows changed, as one
/// command.
#[must_use]
pub fn word_edit_table(args: &WordEditTable, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let retry = call(
        "word_edit_table",
        &[
            ("doc", json!(args.doc)),
            ("table", json!(args.table)),
            ("action", json!(args.action.as_str())),
        ],
    );
    match run(args, editing, &retry) {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

/// `invalid_arguments` at `field`, its way out the options, else the call that would pass.
fn invalid(message: String, field: &str, options: &[&str], retry: &TryCall) -> ToolError {
    let way_out = WayOut::names(options.iter().copied()).unwrap_or_else(|| WayOut::call(retry.clone()));
    ToolError::refusal(Refusal::InvalidArguments, message, way_out).at(field)
}

fn unported(what: &str, retry: &TryCall) -> ToolResult<Value> {
    refused(EditError::Unsupported(what.to_owned()), retry.clone())
}

/// No table `table` in the document (docx-agent's `_no_table`): its body's tables are the
/// options.
fn no_table(docx: &Docx, doc: &str, table: &str, retry: &TryCall) -> ToolError {
    // docx-agent's `Document.tables()`: the body's
    let ids: Vec<String> = docx
        .body()
        .map(|b| b.ids.tables().iter().map(|t| t.id).collect())
        .unwrap_or_default();
    let mut shown: Vec<String> = ids.iter().take(12).cloned().collect();
    if ids.len() > 12 {
        shown.push(format!("and {} more", ids.len() - 12));
    }
    let listed = if ids.is_empty() {
        format!("{doc} has no table")
    } else {
        format!("{doc}'s tables: {}", shown.join(", "))
    };
    let way_out = WayOut::names(ids.iter().take(12).cloned()).unwrap_or_else(|| WayOut::call(retry.clone()));
    ToolError::refusal(
        Refusal::NotFound,
        format!("no table {} in {doc}; {listed}", repr_str(table)),
        way_out,
    )
    .at("table")
}

/// The arguments for another action refused, as docx-agent refuses them before it edits.
fn check_arguments(args: &WordEditTable, retry: &TryCall) -> Result<(), ToolError> {
    // a strict decoder's empty values ("", false, []) are no value (rule 1)
    for (name, given) in [
        ("key", args.key.as_deref().is_some_and(|k| !k.is_empty())),
        ("remove", args.remove == Some(true)),
        ("loose", args.loose.is_some()),
        ("match", args.pairs.as_ref().is_some_and(|p| !p.is_empty())),
    ] {
        if given {
            return Err(invalid(
                format!("{name} is for sync (a table kept in step with rows by key)"),
                name,
                &["sync"],
                retry,
            ));
        }
    }
    for (name, given) in [
        ("from_columns", args.from_columns.is_some()),
        ("header", args.header.is_some()),
        ("date_format", args.date_format.is_some()),
    ] {
        if given {
            return Err(invalid(format!("{name} is for from; give from too"), name, &[], retry));
        }
    }
    if args.grow == Some(true) {
        return Err(invalid(
            "grow adds the rows a source's data needs: it is for set_cells with from; for rows of your own, \
             insert_row"
                .to_owned(),
            "grow",
            &["from", "insert_row"],
            retry,
        ));
    }
    Ok(())
}

fn run(args: &WordEditTable, editing: &Editing<'_>, retry: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let head = editing.editor.store().snapshot();
    let docx = head
        .doc()
        .docx()
        .map_err(|e| crate::review::edit_refusal(e, "", retry.clone()))?;
    let view = match table_view(head.doc(), &docx, &args.table, retry) {
        Ok(view) => view,
        Err(error) if error.code() == ErrorCode::Refusal(Refusal::NotFound) => {
            return Err(no_table(&docx, &args.doc, &args.table, retry))
        }
        Err(error) => return Err(error),
    };
    match args.action {
        TableAction::Sync => return Ok(unported("word_edit_table sync", retry)),
        TableAction::SetCells => return Ok(unported("word_edit_table set_cells", retry)),
        _ => {}
    }
    if args.from.as_deref().is_some_and(|f| !f.is_empty()) {
        // docx-agent reads a `from` before it refuses it for another action
        return Ok(unported("word_edit_table from (sources)", retry));
    }
    check_arguments(args, retry)?;
    let below = args.side != Some(Side::Before);
    let index = |value: Option<i64>| value.map(|v| usize::try_from(v).unwrap_or(usize::MAX));
    let change = match args.action {
        TableAction::InsertRow => TableChange::InsertRow {
            index: index(args.index),
            below,
            plain: true,
        },
        TableAction::DeleteRow => TableChange::DeleteRow {
            index: index(Some(args.index.ok_or_else(|| needed("index", "", retry.clone()))?)).unwrap_or(usize::MAX),
        },
        TableAction::DeleteColumn => {
            let given = args.index.ok_or_else(|| needed("index", "", retry.clone()))?;
            match column(&view, &kept_columns(&docx, &view), given, retry) {
                Column::Grid(index) => TableChange::DeleteColumn { index },
                Column::Off(error) => return Err(error),
                Column::Accepted => {
                    return Ok(unported(
                        "word_edit_table column positions in a table as accepted (a grid column no cell covers)",
                        retry,
                    ))
                }
            }
        }
        TableAction::CopyRow => TableChange::CopyRow {
            index: index(args.index),
            below,
        },
        TableAction::InsertColumn => {
            // beside the column named, as accepted (the last by default)
            let kept = kept_columns(&docx, &view);
            let given = args.index.unwrap_or_else(|| i64::try_from(kept.len()).unwrap_or(0) - 1);
            match column(&view, &kept, given, retry) {
                Column::Grid(index) => TableChange::InsertColumn { index, right: below },
                Column::Off(error) => return Err(error),
                Column::Accepted => {
                    return Ok(unported(
                        "word_edit_table column positions in a table as accepted (a grid column no cell covers)",
                        retry,
                    ))
                }
            }
        }
        TableAction::Merge | TableAction::Split => match cells_change(args, &view, retry)? {
            Some(change) => change,
            None => {
                return Ok(unported(
                    "word_edit_table merge and split positions in a table as accepted (pending tracked structure)",
                    retry,
                ))
            }
        },
        other => return Ok(unported(&format!("word_edit_table {}", other.as_str()), retry)),
    };
    Ok(finish(args, editing, &view, change, retry))
}

/// A merge's or split's change (`_corner`: positions as the table is, with no pending
/// tracked structure); `None` where the table's accepted view counts otherwise.
fn cells_change(args: &WordEditTable, view: &TableView, retry: &TryCall) -> Result<Option<TableChange>, ToolError> {
    let place = |p: &Position| {
        (
            usize::try_from(p.row).unwrap_or(usize::MAX),
            usize::try_from(p.col).unwrap_or(usize::MAX),
        )
    };
    let count = |n: i64| usize::try_from(n).unwrap_or(usize::MAX);
    if args.action == TableAction::Merge {
        let first = args.first.as_ref().ok_or_else(|| needed("first", "", retry.clone()))?;
        let last = args.last.as_ref().ok_or_else(|| needed("last", "", retry.clone()))?;
        if view.pending() {
            return Ok(None);
        }
        return Ok(Some(TableChange::Merge {
            first: place(first),
            last: place(last),
        }));
    }
    let first = args
        .first
        .as_ref()
        .ok_or_else(|| needed("first", " (the cell to split)", retry.clone()))?;
    if view.pending() {
        return Ok(None);
    }
    let (row, column) = place(first);
    let columns = args.columns.filter(|c| *c != 0).map(count);
    no_split_past_span(args, view, first, columns)?;
    Ok(Some(TableChange::Split {
        row,
        column,
        rows: args.rows.map_or(1, count),
        columns,
    }))
}

/// A cell over several grid columns split into more cells than it spans, refused
/// (`_no_split_past_span`, round 57): Word would add grid columns, changing every other
/// row's cells; into 2 up to its span keeps the grid.
fn no_split_past_span(args: &WordEditTable, view: &TableView, at: &Position, columns: Option<usize>) -> Result<(), ToolError> {
    let Some(columns) = columns else { return Ok(()) };
    let row = usize::try_from(at.row).unwrap_or(usize::MAX);
    let column = usize::try_from(at.col).unwrap_or(usize::MAX);
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
    let options = WayOut::options(keeps.iter().map(|k| json!(k)).collect())
        .unwrap_or_else(|| WayOut::one(json!(span)));
    let call = json!({"tool": "word_edit_table", "arguments": {"doc": args.doc, "table": view.table.id,
        "action": "split", "first": {"row": at.row, "col": at.col}, "columns": span}});
    Err(ToolError::refusal(Refusal::Refused, message, options)
        .at("columns")
        .with_detail("call", call))
}

/// A column position against the table (docx-agent's `_Accepted.to_grid`).
enum Column {
    Grid(usize),
    Off(ToolError),
    /// The table's columns as accepted differ from its grid's.
    Accepted,
}

fn column(view: &TableView, kept: &[usize], given: i64, retry: &TryCall) -> Column {
    let count = view.column_count();
    if kept != (0..count).collect::<Vec<_>>() {
        return Column::Accepted;
    }
    match usize::try_from(given).ok().filter(|c| *c < count) {
        Some(index) => Column::Grid(index),
        None => Column::Off(
            ToolError::refusal(
                Refusal::NotFound,
                format!(
                    "index {given}: {} has columns 0 to {}",
                    view.table.id,
                    i64::try_from(count).unwrap_or(0) - 1
                ),
                WayOut::names((0..count.min(50)).map(|c| c.to_string())).unwrap_or_else(|| WayOut::call(retry.clone())),
            )
            .at("index"),
        ),
    }
}

/// The watch on the file's text a deletion takes out of the final view.
fn watch<'a>(editing: &Editing<'_>, now: &'a WordDoc, view: &TableView, change: TableChange) -> Option<Watch<'a>> {
    let opened = editing.opened.docx().ok()?;
    let docx = now.docx().ok()?;
    let mut watch = Watch::new(opened, now);
    match change {
        TableChange::DeleteRow { index } => watch.row(&docx, &view.part, &view.table, index),
        TableChange::DeleteColumn { index } => watch.column(&docx, &view.part, &view.table, index),
        TableChange::InsertRow { .. }
        | TableChange::CopyRow { .. }
        | TableChange::InsertColumn { .. }
        | TableChange::Merge { .. }
        | TableChange::Split { .. } => return None,
    }
    Some(watch)
}

/// Run the change as one command and reply as `_edit_table` does.
fn finish(
    args: &WordEditTable,
    editing: &Editing<'_>,
    view: &TableView,
    change: TableChange,
    retry: &TryCall,
) -> ToolResult<Value> {
    let table = view.table.id.clone();
    let off_before = view.off_grid();
    let before = editing.editor.store().snapshot();
    let mut watched = watch(editing, before.doc(), view, change);
    let command = EditTable {
        table: args.table.clone(),
        change,
        policy: policy(editing),
    };
    let done = match editing.editor.run(editing.caller, &command) {
        Ok(done) => done,
        Err(EditError::Refused(message)) if matches!(change, TableChange::CopyRow { .. }) => {
            return ToolResult::failed(uncopied(args, view, message, change));
        }
        Err(error) => return refused(error, retry.clone()),
    };
    let now = done.out.table.clone();
    // round 55 (R115, rule 3): a merge already made changes nothing
    let unchanged = done.out.unchanged && matches!(change, TableChange::Merge { .. });
    let head = editing.editor.store().snapshot();
    let data = head
        .doc()
        .docx()
        .ok()
        .and_then(|docx| table_json(&docx, &now, head.doc(), retry));
    let summary = format!("{} on {table}", args.action.as_str());
    let mut result = outcome(&[Outcome::from(done.out)], summary, data);
    if unchanged {
        result.warnings.insert(
            0,
            "nothing changed: the cells are merged already (a pending tracked merge counts as made)".to_owned(),
        );
    }
    if let Some(watched) = &mut watched {
        add(&mut result, watched.report(head.doc(), editing.doc));
    }
    let off_now = head
        .doc()
        .docx()
        .ok()
        .and_then(|docx| table_view(head.doc(), &docx, &now, retry).ok())
        .is_some_and(|v| v.off_grid());
    if !off_before && off_now {
        // round 58 (R128): the call is not kept for check's `made_by` yet (the session's)
        result.warnings.push(format!(
            "{now} is off its grid now (a row's cells do not reach across its columns: check's \
             table_grid_mismatch); this call made it so (undo reverses it)"
        ));
    }
    if let ooxml_tools::Outcome::Done(found) = &mut result.outcome {
        if !unchanged && !found.changed.contains(&now) {
            found.changed.push(now.clone());
        }
        if now != table {
            match found.renamed.iter_mut().find(|(old, _)| *old == table) {
                Some(entry) => entry.1.clone_from(&now),
                None => found.renamed.push((table, now)),
            }
        }
    }
    result
}

/// A row `copy_row` does not copy, refused with the calls that add a row like it and fill
/// it (docx-agent's `_copy_row`, rule 2).
fn uncopied(args: &WordEditTable, view: &TableView, message: String, change: TableChange) -> ToolError {
    let TableChange::CopyRow { index, below } = change else {
        return ToolError::internal(message, "word:edit_table");
    };
    let at = index.map_or_else(
        || i64::try_from(view.table.rows.len()).unwrap_or(0) - 1,
        |i| i64::try_from(i).unwrap_or(i64::MAX),
    );
    let side = if below { "after" } else { "before" };
    let insert = call(
        "word_edit_table",
        &[
            ("doc", json!(args.doc)),
            ("table", json!(args.table)),
            ("action", json!("insert_row")),
            ("index", json!(at)),
            ("side", json!(side)),
        ],
    );
    let fill = json!({"tool": "word_edit_table", "arguments": {"doc": args.doc, "table": args.table,
        "action": "set_cells", "cells": [{"row": if below { at + 1 } else { at }, "col": 0, "text": "..."}]}});
    let first = json!({"tool": "word_edit_table", "arguments": {"doc": args.doc, "table": args.table,
        "action": "insert_row", "index": at, "side": side}});
    ToolError::refusal(Refusal::Refused, message, WayOut::call(insert))
        .at("index")
        .with_detail("calls", json!([first, fill]))
}

/// The table's id, size as accepted and text as plain Markdown (docx-agent's `table_json`).
fn table_json(docx: &Docx, id: &str, doc: &ooxml_word_edit::WordDoc, retry: &TryCall) -> Option<Value> {
    let view = table_view(doc, docx, id, retry).ok()?;
    let deleted = deleted_rows(&view.table);
    let kept = kept_columns(docx, &view);
    let mut data = Map::new();
    data.insert("id".into(), json!(view.table.id));
    data.insert("rows".into(), json!(view.table.rows.len() - deleted.len()));
    data.insert("columns".into(), json!(kept.len()));
    let pending: Vec<usize> = (0..view.column_count()).filter(|c| !kept.contains(c)).collect();
    if !pending.is_empty() {
        data.insert("columns_deleted_pending".into(), json!(pending));
    }
    if !deleted.is_empty() {
        data.insert("rows_deleted_pending".into(), json!(deleted));
    }
    data.insert(
        "markdown".into(),
        json!(to_plain_markdown(docx, Some(&view.table.id)).ok()?),
    );
    Some(Value::Object(data))
}

/// The grid columns accepting the table's tracked changes keeps (`accepted_columns`).
fn kept_columns(docx: &Docx, view: &TableView) -> Vec<usize> {
    docx.part(&view.part).map_or_else(
        || (0..view.column_count()).collect(),
        |part| TableGrid::new(part, &view.table).kept_columns(),
    )
}

/// The rows deleted as a tracked change (`w:trPr/w:del`), by place.
fn deleted_rows(table: &TableId) -> Vec<usize> {
    table
        .rows
        .iter()
        .enumerate()
        .filter(|(_, (path, _))| {
            ooxml_word_model::docx::at_path(&table.element, path.get(table.path.len()..).unwrap_or_default())
                .is_some_and(|row| find_path(row, &["trPr", "del"]).is_some_and(|d| d.name().is(Some(W), "del")))
        })
        .map(|(k, _)| k)
        .collect()
}
