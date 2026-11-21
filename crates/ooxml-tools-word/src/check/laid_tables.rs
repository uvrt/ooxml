//! Owns: `check`'s layout facts about tables, drawings past the margins and pages laid
//! out uncertainly, as docx-agent words them in `layout` (`split_rows`,
//! `header_not_repeated`, `column_fit`, `past_margins`, `uncertain`): each entry's JSON,
//! with the call that fixes it.
//!
//! Must not: find the facts (`ooxml_word_pages::layout_facts` does) or filter them.

use ooxml_word_pages::layout_facts::columns::{ColumnFit, FitColumn};
use ooxml_word_pages::layout_facts::margins::{Past, PastMargins};
use ooxml_word_pages::layout_facts::tables::{SplitRow, Unrepeated};
use ooxml_word_pages::layout_facts::uncertain::Uncertain;
use serde_json::{json, Map, Value};

/// A `split_rows` entry.
#[must_use]
pub fn split_row_json(entry: &SplitRow) -> Value {
    json!({"table": entry.table, "row": entry.row, "pages": entry.pages})
}

/// A `header_not_repeated` entry: where the table marks no header row, the
/// `word_format_table` call that marks one (`fix`, or `fix_if_header` where its first row
/// looks like the rest).
#[must_use]
pub fn unrepeated_json(entry: &Unrepeated) -> Value {
    match entry {
        Unrepeated::Unmarked { table, pages, looks } => {
            let fix = json!({"tool": "word_format_table",
                             "calls": [{"table": table, "scope": "table", "header_rows": 1}]});
            let key = if looks.is_empty() { "fix_if_header" } else { "fix" };
            let mut out = json!({"table": table, "pages": pages, "header_rows": 0, "header_look": looks});
            if let Some(object) = out.as_object_mut() {
                object.insert(key.into(), fix);
            }
            out
        }
        Unrepeated::Missing {
            table,
            pages,
            header_rows,
            not_repeated_on,
        } => json!({"table": table, "pages": pages, "header_rows": header_rows, "not_repeated_on": not_repeated_on}),
    }
}

/// A `column_fit` entry, with the `word_format_table` calls setting the two columns' widths.
#[must_use]
pub fn column_fit_json(entry: &ColumnFit) -> Value {
    // docx-agent's key order: a wrapping column's lines after its width
    let column = |c: &FitColumn, lines: bool| {
        let mut out = Map::new();
        out.insert("column".into(), Value::from(c.column));
        out.insert("width".into(), Value::from(c.width));
        if lines {
            out.insert("lines".into(), Value::from(c.lines));
        }
        out.insert("widest_line".into(), Value::from(c.widest_line));
        out.insert("longest_word".into(), Value::from(c.longest_word));
        Value::Object(out)
    };
    let calls: Vec<Value> = [entry.roomy.column, entry.wraps.column]
        .into_iter()
        .map(|c| {
            json!({"table": entry.table, "scope": "column", "column": c,
                   "width": entry.widths.get(c).copied().unwrap_or_default()})
        })
        .collect();
    json!({
        "table": entry.table, "roomy": column(&entry.roomy, false), "wraps": column(&entry.wraps, true),
        "widths": entry.widths, "pages": entry.pages,
        "fix": {"tool": "word_format_table", "calls": calls},
    })
}

fn past_json(past: &Past) -> Value {
    Value::Object(
        past.iter()
            .map(|(side, amount)| ((*side).to_owned(), Value::from(*amount)))
            .collect(),
    )
}

/// A `past_margins` entry, with the `word_drawings` resize that fits an inline picture.
#[must_use]
pub fn past_margins_json(entry: &PastMargins) -> Value {
    match entry {
        PastMargins::Drawing {
            drawing,
            page,
            past,
            fit,
        } => {
            let mut out = json!({"drawing": drawing, "page": page, "past": past_json(past)});
            if let (Some(width), Some(object)) = (fit, out.as_object_mut()) {
                let call = json!({"action": "resize", "target": drawing, "width": width});
                object.insert("fix".into(), json!({"tool": "word_drawings", "calls": [call]}));
            }
            out
        }
        PastMargins::Table { table, page, past } => json!({"table": table, "page": page, "past": past_json(past)}),
    }
}

/// An `uncertain` entry.
#[must_use]
pub fn uncertain_json(entry: &Uncertain) -> Value {
    let mut out = Map::new();
    out.insert("page".into(), Value::from(entry.page));
    out.insert(
        "approximations".into(),
        entry
            .approximations
            .iter()
            .map(|a| json!({"reason": a.reason, "at": a.at}))
            .collect(),
    );
    Value::Object(out)
}
