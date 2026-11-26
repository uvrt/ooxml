//! Owns: `word_insert_markdown`'s `from` naming one field of one row (round 45, R65;
//! docx-agent's `field_text.field_from` and `text_field`): that value's text -- its own
//! Markdown where its source holds one -- written as the `markdown` argument is, verbatim,
//! and kept as a value written by reference; a value that is not text refused with the way
//! out (a figure goes into text as `{{d:...}}`) and the text fields of its row; `from_columns`,
//! `header` and `date_format` refused beside it. The host read the value
//! ([`ooxml_tools::written::OneValue`], its text and record a [`Written`] at `from`, or its
//! refusal).
//!
//! Must not: resolve a source (the host does), or write the Markdown (the tool does).

use super::insert_markdown::WordInsertMarkdown;
use crate::review::Editing;
use crate::Written;
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::sources::Cell;
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};

/// Column types whose values are not text.
const NOT_TEXT: [&str; 6] = ["number", "percent", "date", "datetime", "time", "bool"];

/// The call with its `from` naming one text field read as its Markdown, and the value
/// written (`field_from`); `None` where `from` names rows (a table) or is not given.
///
/// # Errors
/// The host's refusal of the value (or, for a source that is no dataset, of its rows); a
/// value that is not text; `from_columns`, `header` or `date_format` beside one field.
pub(super) fn field_from(
    args: &WordInsertMarkdown,
    editing: &Editing<'_>,
    retry: &TryCall,
) -> Result<Option<(WordInsertMarkdown, Written)>, ToolError> {
    let Some(source) = args.source.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    // the host read one value, refused it, or refused the rows of a source that is no
    // dataset where it could not read one value either
    if let Some(refused) = editing.others.refused_value("from").or_else(|| {
        editing
            .others
            .refused_source(source)
            .filter(|_| !source.starts_with("d:"))
    }) {
        return Err(crate::sources::value_refusal(refused, retry));
    }
    let (Some(value), Some(written)) = (
        editing.others.one_value("from"),
        editing.written.iter().find(|w| w.field == "from"),
    ) else {
        return Ok(None);
    };
    let cell = value.rows.value(value.row, value.column);
    let kind = value.rows.type_of(value.column);
    if !is_blank(cell) && !is_text(cell, kind) {
        return Err(not_text(&value.rows, value.row, cell, kind, &value.shown, retry));
    }
    for (name, given) in [
        ("from_columns", args.from_columns.is_some()),
        ("header", args.header.is_some()),
        ("date_format", args.date_format.is_some()),
    ] {
        if given {
            return Err(ToolError::refusal(
                Refusal::InvalidArguments,
                format!(
                    "{name} is for a table of rows; from {source} is one text field, written as Markdown: leave it out"
                ),
                WayOut::call(retry.clone()),
            )
            .at(name));
        }
    }
    // the value written as its Markdown where its source holds one (`text_field`'s
    // `value_markdown(...) or text`): all of the text the call writes
    let text = written.markdown.clone().unwrap_or_else(|| written.text.clone());
    let mut brief = written.brief.clone();
    brief.insert("text".into(), serde_json::Value::from(text.as_str()));
    let value = Written {
        end: text.chars().count(),
        text: text.clone(),
        markdown: None,
        brief,
        ..written.clone()
    };
    let mut read = args.clone();
    read.markdown = Some(text);
    read.source = None;
    Ok(Some((read, value)))
}

/// The refusal of a value that is not text: the row's text fields, and the way to write a
/// figure.
fn not_text(
    rows: &crate::SourceRows,
    row: usize,
    cell: &Cell,
    kind: Option<&str>,
    shown: &str,
    retry: &TryCall,
) -> ToolError {
    let texts: Vec<&str> = rows
        .columns
        .iter()
        .enumerate()
        .filter(|(k, _)| is_text(rows.value(row, *k), rows.type_of(*k)))
        .map(|(_, c)| c.as_str())
        .collect();
    let what = kind.map_or_else(|| kind_of(cell), str::to_owned);
    let prefix = shown.rsplit_once('/').map_or(shown, |(p, _)| p);
    let listed = if texts.is_empty() {
        String::new()
    } else {
        format!(" ({})", texts.iter().take(6).copied().collect::<Vec<_>>().join(", "))
    };
    let message = format!(
        "{shown} is a {what} value ({}), not text: Markdown comes from a text field{listed}; a figure goes into the \
         markdown as {{{{{shown}}}}}",
        repr(cell)
    );
    let mut options: Vec<String> = texts
        .iter()
        .take(10)
        .map(|c| format!("{prefix}/{}", crate::objects::quote_part(c)))
        .collect();
    options.push(format!("markdown: '{{{{{shown}}}}}'"));
    ToolError::refusal(
        Refusal::InvalidArguments,
        message,
        WayOut::names(options).unwrap_or_else(|| WayOut::call(retry.clone())),
    )
    .at("from")
}

fn is_blank(cell: &Cell) -> bool {
    match cell {
        Cell::Blank => true,
        Cell::Text(text) => text.is_empty(),
        _ => false,
    }
}

fn is_text(cell: &Cell, kind: Option<&str>) -> bool {
    matches!(cell, Cell::Text(_)) && !kind.is_some_and(|k| NOT_TEXT.contains(&k))
}

/// A value's kind as docx-agent names one (`_kind`).
fn kind_of(cell: &Cell) -> String {
    match cell {
        Cell::Bool(_) => "logical".to_owned(),
        Cell::Int(_) | Cell::Float(_) => "number".to_owned(),
        Cell::Date(_) | Cell::DateTime(..) | Cell::Time(_) => "date".to_owned(),
        Cell::Text(_) => "str".to_owned(),
        Cell::Blank => "NoneType".to_owned(),
        Cell::Other(what) => what.clone(),
    }
}

/// A value as Python's `repr` writes it.
fn repr(cell: &Cell) -> String {
    match cell {
        Cell::Blank => "None".to_owned(),
        Cell::Text(text) => repr_str(text),
        Cell::Bool(b) => if *b { "True" } else { "False" }.to_owned(),
        Cell::Int(digits) => digits.clone(),
        Cell::Float(f) => ooxml_base::pyfmt::repr_float(*f),
        Cell::Date(d) => format!("datetime.date({}, {}, {})", d.year, d.month, d.day),
        Cell::DateTime(d, c) => {
            let mut out = format!(
                "datetime.datetime({}, {}, {}, {}, {}",
                d.year, d.month, d.day, c.hour, c.minute
            );
            if c.second != 0 || c.micro != 0 {
                out.push_str(&format!(", {}", c.second));
            }
            if c.micro != 0 {
                out.push_str(&format!(", {}", c.micro));
            }
            out.push(')');
            out
        }
        Cell::Time(c) => {
            let mut out = format!("datetime.time({}, {}", c.hour, c.minute);
            if c.second != 0 || c.micro != 0 {
                out.push_str(&format!(", {}", c.second));
            }
            if c.micro != 0 {
                out.push_str(&format!(", {}", c.micro));
            }
            out.push(')');
            out
        }
        Cell::Other(what) => what.clone(),
    }
}
