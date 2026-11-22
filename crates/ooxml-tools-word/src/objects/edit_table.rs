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
//! Positions count the table as accepting its tracked changes leaves it (docx-agent's
//! `_Accepted`, [`super::table_view::AcceptedView`]): a row or column whose deletion is
//! pending not counted, a cell taken into another answering as it; the notes they make
//! said in the reply. Cells merged or split ([`super::edit_table_cells`]), a merge already
//! made answered as changing nothing (rule 3).
//!
//! Cells written without a source ([`super::set_cells`]).
//!
//! Tracked, a row or column deleted that holds a pending tracked merge's first cell is
//! refused naming the merge's changes to settle first ([`super::edit_table_pending`]).
//!
//! Not ported yet, said in `unported` for the host to answer its own way: `sync` and `from`
//! (a source the dispatcher resolves: the session's datasets and other documents' rows); a
//! change of the rows of a table with a placement record (refused so by the edit core);
//! any call on a table with Accept All's
//! cell ends (a cell's last paragraph mark deleted: the reply's Markdown joins cells across
//! it, and names the cells Accept All loses with the changes that do it).
//!
//! Must not: change the document but through the command.

use super::edit_table_args::{Side, TableAction, WordEditTable};
use super::edit_table_pending::no_reshape_over_merge;
use super::table_view::{table_view, AcceptedView, TableView};
use crate::originals::{add, Watch};
use crate::review::{needed, Editing};
use crate::structure::common::{call, outcome, policy, refused, Outcome};
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{ArgEnum, ErrorCode, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::facts::{ragged_rows, RaggedRow};
use ooxml_word_edit::WordDoc;
use ooxml_word_edit::{EditError, EditTable, Reshape, TableChange, TableChanged};
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

pub(super) fn unported(what: &str, retry: &TryCall) -> ToolResult<Value> {
    refused(EditError::Unsupported(what.to_owned()), retry.clone())
}

/// No table `table` in the document (docx-agent's `_no_table`): its body's tables are the
/// options, and a table of another open document the host holds (a workbook's `t:stock`)
/// is said to be one, with the call that keeps a table here in step with its rows.
fn no_table(docx: &Docx, doc: &str, table: &str, others: crate::Others<'_>, retry: &TryCall) -> ToolError {
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
    let message = match others.host_table(table) {
        // docx-agent's example call, as Python writes a dict
        Some(elsewhere) => format!(
            "{table} is a table of {} ({}), not of {doc}: word_edit_table changes a table of {doc} ({}); to keep \
             one in step with its rows, e.g. {{'table': {}, 'action': 'sync', 'from': {}}}",
            elsewhere.doc,
            elsewhere.kind,
            if shown.is_empty() {
                "it has none".to_owned()
            } else {
                shown.join(", ")
            },
            repr_str(ids.first().map_or("<id>", String::as_str)),
            repr_str(&elsewhere.name),
        ),
        None => format!("no table {} in {doc}; {listed}", repr_str(table)),
    };
    ToolError::refusal(Refusal::NotFound, message, way_out).at("table")
}

/// The arguments for another action refused, as docx-agent refuses them before it edits.
/// Refuse what belongs to `sync` given to another action (docx-agent reads these before
/// `from`).
fn check_sync_fields(args: &WordEditTable, retry: &TryCall) -> Result<(), ToolError> {
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
    Ok(())
}

/// The rows a `from` names, as the host resolved them before the call (its dispatcher
/// refuses an unknown dataset, whatever the action); `None` without a `from`, or for one
/// naming one value (a cell's, by reference: the host's). A `from` the host gave no rows for
/// is refused as one naming nothing.
fn from_rows<'e>(
    args: &WordEditTable,
    editing: &Editing<'e>,
    retry: &TryCall,
) -> Result<Option<&'e crate::SourceRows>, ToolError> {
    let Some(given) = args.from.as_deref().filter(|f| !f.is_empty()) else {
        return Ok(None);
    };
    if let Some(refused) = editing.others.refused_source(given) {
        let head = editing.editor.store().snapshot();
        let docx = head.doc().docx().ok();
        return Err(crate::sources::refused_source(refused, docx.as_deref(), retry));
    }
    match editing.others.source(given) {
        Some(rows) => Ok(Some(rows)),
        None if names_one_value(given) => Ok(None),
        None => Err(crate::sources::no_rows(given, "from", retry)),
    }
}

/// Whether a `from` names one value of one row rather than rows (docx-agent's
/// `field_text.names_one_value`): a dataset address past its id that is no run of rows
/// (`d:notes/#12/body`, `d:notes/Doors/body`).
fn names_one_value(given: &str) -> bool {
    let Some(rest) = given.strip_prefix("d:") else {
        return false;
    };
    let Some((_, after)) = rest.split_once('/') else {
        return false;
    };
    // a run of rows: `#9`, `#9:`, `#9:#16`, `#9:16`
    let digits = |t: &str| t.starts_with(|c: char| ('1'..='9').contains(&c)) && t.bytes().all(|c| c.is_ascii_digit());
    let rows = after.strip_prefix('#').is_some_and(|run| match run.split_once(':') {
        Some((first, last)) => digits(first) && (last.is_empty() || digits(last.strip_prefix('#').unwrap_or(last))),
        None => digits(run),
    });
    !rows
}

/// Refuse what belongs to `from` given without it, and `grow` without a source.
fn check_arguments(args: &WordEditTable, retry: &TryCall) -> Result<(), ToolError> {
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
    let source = from_rows(args, editing, retry)?;
    let head = editing.editor.store().snapshot();
    let docx = head
        .doc()
        .docx()
        .map_err(|e| crate::review::edit_refusal(e, "", retry.clone()))?;
    let view = match table_view(head.doc(), &docx, &args.table, retry) {
        Ok(view) => view,
        Err(error) if error.code() == ErrorCode::Refusal(Refusal::NotFound) => {
            return Err(no_table(&docx, &args.doc, &args.table, editing.others, retry))
        }
        Err(error) => return Err(error),
    };
    if args.action == TableAction::Sync {
        return Ok(unported("word_edit_table sync", retry));
    }
    check_sync_fields(args, retry)?;
    let given = args.from.as_deref().filter(|f| !f.is_empty());
    if let Some(given) = given {
        if args.action != TableAction::SetCells {
            return Err(invalid(
                format!("{} takes no from; from is for set_cells", args.action.as_str()),
                "from",
                &["set_cells"],
                retry,
            ));
        }
        if source.is_none() {
            return Ok(unported(
                "word_edit_table set_cells values by reference (from, {{...}})",
                retry,
            ));
        }
        if args.cells.is_some() {
            return Err(rows_beside_cells(given, retry));
        }
    } else {
        check_arguments(args, retry)?;
    }
    let mut accepted = view.accepted(&docx);
    if accepted.table.cell_ends {
        // the reply's Markdown joins cells across them, and says the cells Accept All loses
        return Ok(unported(
            &format!("word_edit_table on a table with {}", crate::CELL_ENDS),
            retry,
        ));
    }
    if let Some(rows) = source {
        return super::set_from::set_from(args, editing, &view, &accepted, rows, retry);
    }
    if args.action == TableAction::SetCells {
        return super::set_cells::set_cells(args, editing, &docx, &view, &mut accepted, retry);
    }
    let tracked = editing.tracking.is_some();
    let change = match change(args, (tracked, &docx), &view, &mut accepted, retry)? {
        Ok(change) => change,
        Err(what) => return Ok(unported(&what, retry)),
    };
    Ok(finish(args, editing, &view, change, accepted.notes, retry))
}

/// A `from` naming rows beside `cells`: two forms of `set_cells` at once, refused naming
/// what to drop for each (docx-agent's `_rows_beside_cells`, round 49, R96).
fn rows_beside_cells(given: &str, retry: &TryCall) -> ToolError {
    let dataset = given.starts_with("d:");
    let base = if dataset {
        given.split('/').next().unwrap_or(given)
    } else {
        given
    };
    let one = if dataset {
        format!("{base}/#1/<column>")
    } else {
        format!("{base}/<row>/<column>")
    };
    let example = format!("cells: [{{row: 1, col: 2, from: '{one}'}}]");
    invalid(
        format!(
            "from {given} names rows, which fill the table from index, and cells are given too: to fill the table \
             from {given}, drop cells; to set single cells, drop the top-level from and give each cell its own \
             value ({example})"
        ),
        "cells",
        &[given, &example],
        retry,
    )
}

/// The change a row, column, merge or split action names, positions as the table is
/// accepted; `Err` with what `unported` says where it is not ported yet.
fn change(
    args: &WordEditTable,
    (tracked, docx): (bool, &Docx),
    view: &TableView,
    accepted: &mut AcceptedView,
    retry: &TryCall,
) -> Result<Result<TableChange, String>, ToolError> {
    let below = args.side != Some(Side::Before);
    let height = i64::try_from(accepted.table.height()).unwrap_or(0) - 1;
    // a row named as accepted: one whose deletion is pending stays on its other side
    let row = |accepted: &mut AcceptedView, value: Option<i64>| -> Result<Option<usize>, ToolError> {
        match (accepted.table.rows_active(), value) {
            (true, value) => accepted.document_row(value.unwrap_or(height), "index", retry).map(Some),
            (false, value) => Ok(value.map(|v| usize::try_from(v).unwrap_or(usize::MAX))),
        }
    };
    let change = match args.action {
        TableAction::InsertRow => TableChange::InsertRow {
            index: row(accepted, args.index)?,
            below,
            plain: true,
        },
        TableAction::DeleteRow => {
            let given = args.index.ok_or_else(|| needed("index", "", retry.clone()))?;
            let index = row(accepted, Some(given))?.unwrap_or(usize::MAX);
            if tracked {
                no_reshape_over_merge(&args.doc, docx, view, accepted, Reshape::DeleteRow(index))?;
            }
            TableChange::DeleteRow { index }
        }
        TableAction::DeleteColumn => {
            let given = args.index.ok_or_else(|| needed("index", "", retry.clone()))?;
            let index = accepted.grid_column(given, "index", retry)?;
            if tracked {
                no_reshape_over_merge(&args.doc, docx, view, accepted, Reshape::DeleteColumn(index))?;
            }
            TableChange::DeleteColumn { index }
        }
        TableAction::CopyRow => TableChange::CopyRow {
            index: row(accepted, args.index)?,
            below,
        },
        TableAction::InsertColumn => {
            // beside the column named, as accepted (the last by default)
            let last = i64::try_from(accepted.table.width()).unwrap_or(0) - 1;
            TableChange::InsertColumn {
                index: accepted.grid_column(args.index.unwrap_or(last), "index", retry)?,
                right: below,
            }
        }
        TableAction::Merge | TableAction::Split => {
            super::edit_table_cells::cells_change(args, (tracked, docx), view, accepted, retry)?
        }
        other => return Ok(Err(format!("word_edit_table {}", other.as_str()))),
    };
    Ok(Ok(change))
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
    notes: Vec<String>,
    retry: &TryCall,
) -> ToolResult<Value> {
    let table = view.table.id.clone();
    let off_before = view.off_grid();
    let before = editing.editor.store().snapshot();
    let ragged_before = !matches!(change, TableChange::InsertRow { .. }) && ragged(before.doc(), &table).is_some();
    let mut watched = watch(editing, before.doc(), view, change);
    let command = EditTable {
        table: args.table.clone(),
        change,
        policy: policy(editing),
    };
    // round 58 (R126): a row added looked at against the rows around it, in the call's
    // transaction
    let adds_row = matches!(change, TableChange::InsertRow { .. } | TableChange::CopyRow { .. });
    let look =
        |out: &TableChanged, _: &WordDoc| adds_row.then(|| (out.table.clone(), out.id.iter().cloned().collect()));
    let (done, rows) = match super::rows_written::run_writing_rows(editing, &command, &table, look, retry) {
        Ok(done) => done,
        Err(EditError::Refused(message)) if matches!(change, TableChange::CopyRow { .. }) => {
            return ToolResult::failed(uncopied(args, view, message, change));
        }
        Err(error) => return refused(error, retry.clone()),
    };
    let now = done.table.clone();
    // round 55 (R115, rule 3): a merge already made changes nothing
    let unchanged = done.unchanged && matches!(change, TableChange::Merge { .. });
    let head = editing.editor.store().snapshot();
    let data = head
        .doc()
        .docx()
        .ok()
        .and_then(|docx| table_json(&docx, &now, head.doc(), true, retry));
    let summary = format!("{} on {table}", args.action.as_str());
    let mut result = outcome(&[Outcome::from(done)], summary, data);
    if unchanged {
        result.warnings.insert(
            0,
            "nothing changed: the cells are merged already (a pending tracked merge counts as made)".to_owned(),
        );
    }
    if let Some(watched) = &mut watched {
        add(&mut result, watched.report(head.doc(), editing.doc));
    }
    result.warnings.extend(notes);
    if !ragged_before && !matches!(change, TableChange::InsertRow { .. }) {
        result.warnings.extend(made_ragged(head.doc(), &now));
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
                None => found.renamed.push((table.clone(), now.clone())),
            }
        }
    }
    result.warnings.extend(rows);
    result
}

/// The table `id` ragged (docx-agent's `_ragged`, `layout_facts.ragged_rows`: its rows'
/// cells not all starting and ending on the same grid columns, as Accept All leaves it
/// where pending changes reshape it -- a table off its grid too): the grid columns its
/// widest rows cover, its other rows (at most check's 20) and whether pending changes make
/// the shape.
fn ragged(doc: &WordDoc, id: &str) -> Option<(usize, Vec<RaggedRow>, bool)> {
    let docx = doc.docx().ok()?;
    let table = docx.body()?.ids.tables().iter().find(|t| t.id == id)?;
    let found = ragged_rows(doc, &table.element)?;
    Some((
        found.grid,
        found.rows.into_iter().take(RAGGED_LISTED).collect(),
        found.after_accept,
    ))
}

/// Rows of a ragged table check lists (`layout_facts.CAP`).
const RAGGED_LISTED: usize = 20;

/// The warning of a change that left `table` ragged where it was not (`_made_ragged`,
/// round 57 R125, rule 7); the call is not kept for check's `made_by` yet (the session's).
fn made_ragged(doc: &WordDoc, table: &str) -> Option<String> {
    let (grid, rows, after_accept) = ragged(doc, table)?;
    let named: Vec<String> = rows
        .iter()
        .take(5)
        .map(|r| format!("{} (columns {}-{})", r.row, r.covers.0, r.covers.1))
        .collect();
    let more = if rows.len() > 5 {
        format!(" and {} more", rows.len() - 5)
    } else {
        String::new()
    };
    let when = if after_accept {
        "once its tracked changes are accepted"
    } else {
        "now"
    };
    Some(format!(
        "{table} is ragged {when}: rows {}{more} end short of the others' {grid} columns; this call made it so \
         (undo reverses it)",
        named.join(", ")
    ))
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

/// The table's id, size as accepted and (`markdown`) text as plain Markdown (docx-agent's
/// `table_json`).
pub(super) fn table_json(
    docx: &Docx,
    id: &str,
    doc: &ooxml_word_edit::WordDoc,
    markdown: bool,
    retry: &TryCall,
) -> Option<Value> {
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
    if markdown {
        data.insert(
            "markdown".into(),
            json!(to_plain_markdown(docx, Some(&view.table.id)).ok()?),
        );
    }
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
pub(super) fn deleted_rows(table: &TableId) -> Vec<usize> {
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

/// Registers `word_edit_table` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordEditTable>(word_edit_table, crate::Hosting::SERVED);
}
