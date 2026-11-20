//! Owns: `word_format_table`'s handler (docx-agent's `tools/objects.py`,
//! `word_format_table`): its arguments read into the edits docx-agent's `Document` methods
//! make -- a table's properties, its rows' breaking, a column's width, a row's height, the
//! cells of a row, column or one cell -- run as one command, and the reply.
//!
//! Must not: change the document but through the command.

use super::format_table_args::{BorderStyle, HeightRule, Look, Scope, WidthOf, WordFormatTable};
use super::table_view::{regions_off, table_view, TableView};
use crate::review::{needed, Editing};
use crate::structure::common::{call, outcome, policy, refused, Outcome};
use ooxml_tools::{ArgEnum, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{CellValues, EditError, FormatTable, TableEdit, TableFormat, TableValues, TableWidth};
use serde_json::{json, Value};

/// keep_together on a table taller than a page (docx-agent's `TALLER_THAN_PAGE`, round 25):
/// Word does keep it, so does the tool.
const TALLER_THAN_PAGE: &str = "taller than a page: Word starts it on a new page and breaks it where the page is full";
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
    Ok(finish(args, editing, formats, summary, false, retry).result)
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

/// The table's own properties the call sets (`set_table`'s values): on the body's lines,
/// `body`'s width and indent, aligned left.
fn table_values(
    args: &WordFormatTable,
    body: Option<&ooxml_word_edit::BodyWidth>,
    fill: Option<Option<String>>,
    borders: Option<String>,
) -> TableValues {
    TableValues {
        style: args.style.clone(),
        width: match (body, args.width_percent, args.width) {
            (Some(body), _, _) => Some(TableWidth::Points(body.width)),
            // AutoFit to Window takes width_percent (after the rest)
            _ if args.width_of == Some(WidthOf::Window) => None,
            (None, Some(p), _) => Some(TableWidth::Percent(six_digits(p))),
            (None, None, Some(w)) => Some(TableWidth::Points(w)),
            (None, None, None) => None,
        },
        alignment: match body {
            Some(_) => Some("left".to_owned()),
            None => args.alignment.map(|a| a.as_str().to_owned()),
        },
        indent: body.map(|b| b.indent),
        header_rows: args.header_rows.map(|h| usize::try_from(h).unwrap_or(usize::MAX)),
        shading: fill,
        borders,
        look: args.look.as_deref().map(look_of),
    }
}

fn table_scope(
    args: &WordFormatTable,
    editing: &Editing<'_>,
    view: &TableView,
    fill: Option<Option<String>>,
    borders: Option<String>,
    retry: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    let body_or_window = matches!(args.width_of, Some(WidthOf::Body | WidthOf::Window));
    if body_or_window && args.width.is_some() {
        let message = "width_of is what width_percent is of: give width_percent (or nothing, for 100), not width";
        return Err(invalid(message, Some("width_of"), None, retry));
    }
    let body = match args.width_of {
        Some(WidthOf::Body) => Some(on_body_lines(args, editing, retry)?),
        _ => None,
    };
    let values = table_values(args, body.as_ref(), fill, borders);
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
    if args.keep_together.is_some() || args.keep_with_next.is_some() {
        formats.push(TableFormat::Keeps {
            keep_together: args.keep_together,
            keep_with_next: args.keep_with_next,
        });
    }
    if let Some(body) = &body {
        // the columns shared again first, the cells' widths with them: an autofit table is
        // drawn at its cells' widths, which a table width alone leaves
        formats.extend(
            body.columns
                .iter()
                .enumerate()
                .map(|(column, &width)| TableFormat::ColumnWidth { column, width }),
        );
    }
    let has_values = values != TableValues::default();
    if !has_values && formats.is_empty() && args.width_of != Some(WidthOf::Window) {
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
    if args.width_of == Some(WidthOf::Window) {
        // round 48: Word's AutoFit to Window, after a style or borders given with it (the
        // grid follows the style's borders and margins)
        formats.push(TableFormat::Window {
            percent: args.width_percent.unwrap_or(100.0),
        });
    }
    let summary = format!("Formatted {}", args.table);
    let regions = args.style.is_some() || args.look.is_some();
    let Finished {
        mut result,
        changed,
        pages,
    } = finish(args, editing, formats, summary, regions, retry);
    if !has_values && !changed {
        // round 27, W7: asked for what the table already is; the warnings say why
        if let ooxml_tools::Outcome::Done(found) = &mut result.outcome {
            found.summary = format!("No change to {}", args.table);
            found.changed.clear();
        }
    }
    if args.keep_together.is_some() || args.keep_with_next.is_some() {
        keep_notes(args, editing, pages, &mut result);
    }
    Ok(result)
}

/// `width_of: "body"` (round 21, T5): the table on the body's lines beside it
/// ([`ooxml_word_edit::body_width`]), `width_percent` of them (100 unless given); refused
/// where that is under a point.
fn on_body_lines(
    args: &WordFormatTable,
    editing: &Editing<'_>,
    retry: &TryCall,
) -> Result<ooxml_word_edit::BodyWidth, ToolError> {
    let head = editing.editor.store().snapshot();
    let percent = args.width_percent.unwrap_or(100.0);
    let alignment = args.alignment.map(|a| a.as_str());
    let refusal = |e| crate::review::edit_refusal(e, "", retry.clone());
    if let Some(body) = ooxml_word_edit::body_width(head.doc(), &args.table, percent, alignment).map_err(refusal)? {
        return Ok(body);
    }
    let lines = ooxml_word_edit::body_lines(head.doc(), &args.table).map_err(refusal)?;
    let message = format!(
        "the body's lines beside {} are {} pt wide",
        args.table,
        crate::table_read::python_g(lines.width)
    );
    Err(invalid(&message, Some("width_of"), None, retry))
}

/// What a call with `keep_together` or `keep_with_next` says after its edit (docx-agent's
/// `word_format_table`): a table kept together that is taller than a page, or whose pages
/// the layout cannot say (`_taller_than_page`, from `pages`); a loose paragraph between the
/// table kept with the next and the table after it (round 25, W5); else a table released
/// that its rows' styles still keep together.
fn keep_notes(
    args: &WordFormatTable,
    editing: &Editing<'_>,
    pages: Option<Result<Vec<usize>, ooxml_word_edit::Unknown>>,
    result: &mut ToolResult<Value>,
) {
    let ooxml_tools::Outcome::Done(found) = &result.outcome else {
        return;
    };
    let table = found
        .renamed
        .iter()
        .find(|(old, _)| *old == args.table)
        .map_or_else(|| args.table.clone(), |(_, new)| new.clone());
    match pages {
        Some(Ok(pages)) if pages.len() > 1 => {
            result.warnings.push(format!("{table} is {TALLER_THAN_PAGE}"));
        }
        Some(Err(_)) => result.warnings.push(format!(
            "{table} is kept together, but the document could not be laid out to it: whether it is taller than a \
             page is not known (if it is, Word starts it on a new page and breaks it where the page is full)"
        )),
        _ => {}
    }
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

/// What [`finish`] gives: the reply, whether any edit changed the table, and the pages a
/// table kept together is drawn on (the command's [`ooxml_word_edit::TableFormatted::pages`]).
struct Finished {
    result: ToolResult<Value>,
    changed: bool,
    pages: Option<Result<Vec<usize>, ooxml_word_edit::Unknown>>,
}

impl From<ToolResult<Value>> for Finished {
    fn from(result: ToolResult<Value>) -> Self {
        Finished {
            result,
            changed: false,
            pages: None,
        }
    }
}

/// Run `formats` as one command and reply as `outcome` does, the table among the changed;
/// with `regions`, the note where its style formats a region its look turns off.
fn finish(
    args: &WordFormatTable,
    editing: &Editing<'_>,
    formats: Vec<TableFormat>,
    summary: String,
    regions: bool,
    retry: &TryCall,
) -> Finished {
    let command = FormatTable {
        table: args.table.clone(),
        formats,
        policy: policy(editing),
        layout: editing.layout,
    };
    let done = match editing.editor.run(editing.caller, &command) {
        Ok(done) => done,
        Err(error) => return refused(error, retry.clone()).into(),
    };
    let pages = done.out.pages;
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
    Finished { result, changed, pages }
}
