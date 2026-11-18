//! Owns: `word_edit_table`'s handler (docx-agent's `tools/objects.py`, `_edit_table`): its
//! arguments read as docx-agent reads them (what belongs to `sync` or to `from` refused
//! elsewhere, rule 2), a row inserted as one command, and the reply: the edit's
//! ids, the table's id after it (renamed where a row at the top changed), and the table as
//! `table_json` gives it (its size as accepted and its text as plain Markdown).
//!
//! Not ported yet, said in `unported` for the host to answer its own way: `set_cells`,
//! `sync`, `from`, `copy_row`, `delete_row` (its report of the file's text removed), the
//! columns, `merge` and `split`; a table with pending tracked structure or a placement
//! record.
//!
//! Must not: change the document but through the command.

use super::edit_table_args::{Side, TableAction, WordEditTable};
use super::table_view::{table_view, TableView};
use crate::review::Editing;
use crate::structure::common::{call, outcome, policy, refused, Outcome};
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{ArgEnum, ErrorCode, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{EditError, EditTable, TableChange, TableChanged};
use ooxml_word_model::docx::{find_path, Docx};
use ooxml_word_model::ids::{TableId, W};
use ooxml_word_model::markdown::to_plain_markdown;
use serde_json::{json, Map, Value};

/// What `unported` says of a deletion, until its report of the file's text removed is.
const DELETES: &str = "word_edit_table delete_row and delete_column (the report of the file's text removed)";

impl From<TableChanged> for Outcome {
    fn from(edit: TableChanged) -> Self {
        Outcome {
            id: edit.id,
            changed: true,
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

/// No table `table` in the document (docx-agent's `_no_table`): its tables are the options.
fn no_table(docx: &Docx, doc: &str, table: &str, retry: &TryCall) -> ToolError {
    let ids: Vec<String> = docx
        .parts
        .iter()
        .flat_map(|p| p.ids.tables().iter().map(|t| t.id).collect::<Vec<_>>())
        .collect();
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
        // the report of the file's own text a deletion takes out of the final view
        // (docx-agent's `tools/originals.py`) is not ported yet
        TableAction::DeleteRow => return Ok(unported(DELETES, retry)),
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
        other => return Ok(unported(&format!("word_edit_table {}", other.as_str()), retry)),
    };
    Ok(finish(args, editing, &view, change, retry))
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
    let command = EditTable {
        table: args.table.clone(),
        change,
        policy: policy(editing),
    };
    let done = match editing.editor.run(editing.caller, &command) {
        Ok(done) => done,
        Err(error) => return refused(error, retry.clone()),
    };
    let now = done.out.table.clone();
    let head = editing.editor.store().snapshot();
    let data = head
        .doc()
        .docx()
        .ok()
        .and_then(|docx| table_json(&docx, &now, head.doc(), retry));
    let summary = format!("{} on {table}", args.action.as_str());
    let mut result = outcome(&[Outcome::from(done.out)], summary, data);
    if let ooxml_tools::Outcome::Done(found) = &mut result.outcome {
        if !found.changed.contains(&now) {
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

/// The table's id, size as accepted and text as plain Markdown (docx-agent's `table_json`).
fn table_json(docx: &Docx, id: &str, doc: &ooxml_word_edit::WordDoc, retry: &TryCall) -> Option<Value> {
    let view = table_view(doc, docx, id, retry).ok()?;
    let deleted = deleted_rows(&view.table);
    let mut data = Map::new();
    data.insert("id".into(), json!(view.table.id));
    data.insert("rows".into(), json!(view.table.rows.len() - deleted.len()));
    data.insert("columns".into(), json!(view.column_count()));
    if !deleted.is_empty() {
        data.insert("rows_deleted_pending".into(), json!(deleted));
    }
    data.insert(
        "markdown".into(),
        json!(to_plain_markdown(docx, Some(&view.table.id)).ok()?),
    );
    Some(Value::Object(data))
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
