//! Owns: `word_format_table`'s handler (docx-agent's `tools/objects.py`,
//! `word_format_table`): its arguments read into the edits docx-agent's `Document` methods
//! make -- a table's properties, its rows' breaking, a column's width, a row's height, the
//! cells of a row, column or one cell -- run as one command, and the reply.
//!
//! Must not: change the document but through the command.

use super::format_table_args::{BorderStyle, HeightRule, Look, Scope, WordFormatTable};
use super::table_view::{regions_off, table_view, TableView};
use crate::review::{needed, Editing};
use crate::structure::common::{call, outcome, policy, refused, Outcome};
use ooxml_tools::{ArgEnum, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{CellValues, EditError, FormatTable, TableEdit, TableFormat, TableValues, TableWidth};
use serde_json::{json, Value};

/// What `unported` says of a call that needs what is not ported yet.
const LAYOUT: &str = "word_format_table width_of body or window, keep_together true";
/// What `unported` says of a row, column or cell of a table with tracked structure.
const ACCEPTED: &str = "word_format_table positions in a table with pending tracked structure";

impl From<TableEdit> for Outcome {
    fn from(edit: TableEdit) -> Self {
        Outcome {
            id: Some(edit.id),
            changed: edit.changed,
            created: Vec::new(),
            removed: Vec::new(),
            renamed: edit.renamed,
            warnings: edit.warnings,
        }
    }
}

/// `word_format_table` (docx-agent's `word_format_table`): a table, row, column or cell
/// formatted, as one command.
#[must_use]
pub fn word_format_table(args: &WordFormatTable, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let retry = call(
        "word_format_table",
        &[
            ("doc", json!(args.doc)),
            ("table", json!(args.table)),
            ("scope", json!(args.scope.as_str())),
        ],
    );
    match run(args, editing, &retry) {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

fn invalid(message: &str, field: Option<&str>, options: Option<&[&str]>, call: &TryCall) -> ToolError {
    let way_out = options
        .and_then(|o| WayOut::names(o.iter().copied()))
        .unwrap_or_else(|| WayOut::call(call.clone()));
    let error = ToolError::refusal(Refusal::InvalidArguments, message.to_owned(), way_out);
    match field {
        Some(field) => error.at(field),
        None => error,
    }
}

fn unported(what: &str, retry: &TryCall) -> ToolResult<Value> {
    refused(EditError::Unsupported(what.to_owned()), retry.clone())
}

fn run(args: &WordFormatTable, editing: &Editing<'_>, retry: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let head = editing.editor.store().snapshot();
    let docx = head
        .doc()
        .docx()
        .map_err(|e| crate::review::edit_refusal(e, "", retry.clone()))?;
    let view = table_view(head.doc(), &docx, &args.table, retry)?;
    let fill = args.shading.as_ref().map(|s| (s != "none").then(|| s.clone()));
    let borders = args.borders.map(|b| BorderStyle::as_str(b).to_owned());
    if args.scope == Scope::Table {
        return table_scope(args, editing, &view, fill, borders, retry);
    }
    for (name, given, message) in [
        ("look", args.look.is_some(), "look is the table's: give scope table"),
        (
            "keep_together",
            args.keep_together.is_some(),
            "keep_together is the table's: give scope table (cant_split keeps a row on one page)",
        ),
        (
            "keep_with_next",
            args.keep_with_next.is_some(),
            "keep_with_next is the table's: give scope table (word_format keeps a paragraph with the next)",
        ),
        (
            "cant_split",
            args.cant_split.is_some() && args.scope != Scope::Row,
            "cant_split is a row's: give scope row (or table, for every row)",
        ),
    ] {
        if given {
            return Err(invalid(message, Some(name), None, retry));
        }
    }
    if view.pending() {
        return Ok(unported(ACCEPTED, retry));
    }
    let mut values = CellValues {
        shading: fill,
        borders,
        vertical_alignment: args.vertical_alignment.map(|v| v.as_str().to_owned()),
        width: None,
    };
    let (formats, positions) = part_formats(args, &view, &mut values, retry)?;
    let mut formats = formats;
    if values != CellValues::default() {
        let mut seen = Vec::new();
        for (r, c) in positions {
            let Some(origin) = view.origin(r, c) else { continue };
            if seen.contains(&origin) {
                continue;
            }
            seen.push(origin);
            formats.push(TableFormat::Cell {
                row: r,
                column: c,
                values: values.clone(),
            });
        }
    }
    if formats.is_empty() {
        let options = [
            "shading",
            "borders",
            "vertical_alignment",
            "width",
            "height",
            "cant_split",
        ];
        let message = format!("nothing to set for the {}", args.scope.as_str());
        return Err(invalid(&message, None, Some(&options), retry));
    }
    let summary = format!("Formatted {} of {}", args.scope.as_str(), args.table);
    Ok(finish(args, editing, formats, summary, false, retry).0)
}

/// A row's, column's or cell's own edits, and the grid positions whose cells are formatted.
type Planned = (Vec<TableFormat>, Vec<(usize, usize)>);

/// The row's, column's or cell's own edits and the positions whose cells are formatted.
fn part_formats(
    args: &WordFormatTable,
    view: &TableView,
    values: &mut CellValues,
    retry: &TryCall,
) -> Result<Planned, ToolError> {
    let index = |value: Option<i64>, name: &str| {
        value
            .map(|v| usize::try_from(v).unwrap_or(usize::MAX))
            .ok_or_else(|| needed(name, "", retry.clone()))
    };
    let rows = view.table.rows.len();
    let mut formats = Vec::new();
    let positions = match args.scope {
        Scope::Row => {
            let row = index(args.row, "row")?;
            if args.height.is_some() || args.header_rows.is_some() || args.cant_split.is_some() {
                formats.push(TableFormat::Row {
                    row,
                    height: args.height.map(|h| (h, args.height_rule == Some(HeightRule::Exact))),
                    cant_split: args.cant_split,
                });
            }
            (0..view.column_count()).map(|c| (row, c)).collect()
        }
        Scope::Column => {
            let column = index(args.column, "column")?;
            if let Some(width) = args.width {
                formats.push(TableFormat::ColumnWidth { column, width });
            }
            (0..rows).map(|r| (r, column)).collect()
        }
        _ => {
            let row = index(args.row, "row")?;
            let column = index(args.column, "column")?;
            values.width = args.width;
            vec![(row, column)]
        }
    };
    Ok((formats, positions))
}

/// `look` as `w:tblLook`'s six options: on what is given, off the rest.
fn look_of(given: &[Look]) -> ooxml_word_edit::Look {
    let on = |l: Look| given.contains(&l);
    [
        on(Look::HeaderRow),
        on(Look::TotalRow),
        on(Look::FirstColumn),
        on(Look::LastColumn),
        on(Look::BandedRows),
        on(Look::BandedColumns),
    ]
}

/// A percent as Python's `f"{value:g}%"` reads back: six significant digits.
fn six_digits(value: f64) -> f64 {
    format!("{value:.5e}").parse().unwrap_or(value)
}

fn table_scope(
    args: &WordFormatTable,
    editing: &Editing<'_>,
    view: &TableView,
    fill: Option<Option<String>>,
    borders: Option<String>,
    retry: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    use super::format_table_args::WidthOf;
    let body_or_window = matches!(args.width_of, Some(WidthOf::Body | WidthOf::Window));
    if body_or_window && args.width.is_some() {
        let message = "width_of is what width_percent is of: give width_percent (or nothing, for 100), not width";
        return Err(invalid(message, Some("width_of"), None, retry));
    }
    if body_or_window {
        return Ok(unported(LAYOUT, retry));
    }
    let values = TableValues {
        style: args.style.clone(),
        width: match (args.width_percent, args.width) {
            (Some(p), _) => Some(TableWidth::Percent(six_digits(p))),
            (None, Some(w)) => Some(TableWidth::Points(w)),
            (None, None) => None,
        },
        alignment: args.alignment.map(|a| a.as_str().to_owned()),
        header_rows: args.header_rows.map(|h| usize::try_from(h).unwrap_or(usize::MAX)),
        shading: fill,
        borders,
        look: args.look.as_deref().map(look_of),
    };
    let mut formats: Vec<TableFormat> = Vec::new();
    if let Some(on) = args.cant_split {
        formats.extend((0..view.table.rows.len()).map(|row| TableFormat::Row {
            row,
            height: None,
            cant_split: Some(on),
        }));
    }
    if args.keep_together == Some(false) && args.keep_with_next == Some(true) {
        let message = "keep_with_next keeps a table kept together with the next block: give keep_together true \
                       with it, or leave keep_with_next out";
        return Err(invalid(message, Some("keep_with_next"), None, retry));
    }
    if args.keep_together == Some(true) {
        // its "taller than a page" warning reads the layout
        return Ok(unported(LAYOUT, retry));
    }
    if args.keep_together.is_some() || args.keep_with_next.is_some() {
        formats.push(TableFormat::Keeps {
            keep_together: args.keep_together,
            keep_with_next: args.keep_with_next,
        });
    }
    let has_values = values != TableValues::default();
    if !has_values && formats.is_empty() {
        let options = [
            "style",
            "width",
            "width_percent",
            "alignment",
            "shading",
            "borders",
            "header_rows",
            "look",
            "cant_split",
            "keep_together",
            "keep_with_next",
        ];
        return Err(invalid("nothing to set for the table", None, Some(&options), retry));
    }
    if has_values {
        formats.push(TableFormat::Table(values));
    }
    let summary = format!("Formatted {}", args.table);
    let regions = args.style.is_some() || args.look.is_some();
    let (mut result, changed) = finish(args, editing, formats, summary, regions, retry);
    if !has_values && !changed {
        // round 27, W7: asked for what the table already is; the warnings say why
        if let ooxml_tools::Outcome::Done(found) = &mut result.outcome {
            found.summary = format!("No change to {}", args.table);
            found.changed.clear();
        }
    }
    if args.keep_together.is_some() || args.keep_with_next.is_some() {
        keep_notes(args, editing, &mut result);
    }
    Ok(result)
}

/// What a call with `keep_together` or `keep_with_next` says after its edit (docx-agent's
/// `word_format_table`): a loose paragraph
/// between the table kept with the next and the table after it (round 25, W5); else a table
/// released that its rows' styles still keep together.
fn keep_notes(args: &WordFormatTable, editing: &Editing<'_>, result: &mut ToolResult<Value>) {
    let ooxml_tools::Outcome::Done(found) = &result.outcome else {
        return;
    };
    let table = found
        .renamed
        .iter()
        .find(|(old, _)| *old == args.table)
        .map_or_else(|| args.table.clone(), |(_, new)| new.clone());
    let head = editing.editor.store().snapshot();
    let loose = if args.keep_with_next == Some(true) {
        ooxml_word_edit::loose_paragraph_after(head.doc(), &table)
            .ok()
            .flatten()
    } else {
        None
    };
    if let Some((table, paragraph, after)) = loose {
        result.warnings.push(format!(
            "{paragraph}, between {table} and {after}, does not keep with the next: {after} can still start a page \
             without {table}; word_format keep_with_next true on {paragraph} keeps them together"
        ));
    } else if args.keep_together == Some(false) {
        let kept = ooxml_word_edit::table_keeps(head.doc(), &table).is_ok_and(|k| k.together);
        if kept {
            result.warnings.push(format!(
                "{table} is still kept together by its rows' styles (they keep with the next); keep_together \
                 false removes only the direct keeps"
            ));
        }
    }
}

/// Run `formats` as one command and reply as `outcome` does, the table among the changed;
/// with `regions`, the note where its style formats a region its look turns off; and
/// whether any edit changed the table.
fn finish(
    args: &WordFormatTable,
    editing: &Editing<'_>,
    formats: Vec<TableFormat>,
    summary: String,
    regions: bool,
    retry: &TryCall,
) -> (ToolResult<Value>, bool) {
    let command = FormatTable {
        table: args.table.clone(),
        formats,
        policy: policy(editing),
    };
    let done = match editing.editor.run(editing.caller, &command) {
        Ok(done) => done,
        Err(error) => return (refused(error, retry.clone()), false),
    };
    let changed = done.out.edits.iter().any(|e| e.changed);
    let now = done.out.edits.first().map(|e| e.id.clone());
    let edits: Vec<Outcome> = done.out.edits.into_iter().map(Outcome::from).collect();
    let mut result = outcome(&edits, summary, None);
    if let ooxml_tools::Outcome::Done(found) = &mut result.outcome {
        if !found.changed.contains(&args.table) {
            found.changed.push(args.table.clone());
        }
    }
    if let (true, Some(now)) = (regions, now) {
        let head = editing.editor.store().snapshot();
        let note = head.doc().docx().ok().and_then(|docx| {
            let view = table_view(head.doc(), &docx, &now, retry).ok()?;
            regions_off(&docx, &view.table)
        });
        result.warnings.extend(note);
    }
    (result, changed)
}
