//! Owns: `check`'s facts read from the document alone as docx-agent words them in `layout`
//! (`empty_paragraphs`, `numbering_skips_level`, `table_grid_mismatch`, `accept_loses_text`,
//! `direct_formatting`, `styles_not_allowed`):
//! each entry's JSON, with the calls that fix it, an entry's own lists capped as docx-agent
//! caps them.
//!
//! Must not: find the facts (`ooxml_word_edit::facts` does), filter or cap them.

use ooxml_word_edit::facts::{DirectFormatting, EmptyRun, FormatValue, GridMismatch, LostCell, SkippedLevel, StyleUse};
use serde_json::{json, Map, Value};

/// The most of a list inside one entry (docx-agent's `CAP`).
const CAP: usize = 20;

/// The blocks a `styles_not_allowed` entry names at most.
const STYLE_BLOCKS: usize = 10;

/// The first [`CAP`] of `items` as JSON, and how many are left out.
fn capped<T>(items: &[T], each: impl Fn(&T) -> Value) -> (Value, usize) {
    let shown: Vec<Value> = items.iter().take(CAP).map(each).collect();
    (Value::from(shown), items.len().saturating_sub(CAP))
}

/// A `table_grid_mismatch` entry: a table off its grid (`drawn` `widened` or `short`, with
/// the changes that put it there and the review that settles only them) or ragged.
#[must_use]
pub fn grid_mismatch_json(entry: &GridMismatch) -> Value {
    let mut out = Map::new();
    match entry {
        GridMismatch::OffGrid {
            table,
            grid,
            rows,
            widened,
            tracked,
            changes,
            fix,
        } => {
            let (shown, left) = capped(rows, |r| json!({"row": r.row, "reaches": r.reaches}));
            out.insert("table".into(), Value::from(table.clone()));
            out.insert("grid".into(), Value::from(*grid));
            out.insert("rows".into(), shown);
            out.insert("drawn".into(), Value::from(if *widened { "widened" } else { "short" }));
            if left > 0 {
                out.insert("rows_left_out".into(), Value::from(left));
            }
            if *tracked {
                out.insert("tracked".into(), Value::Bool(true));
                if !changes.is_empty() {
                    out.insert("changes".into(), capped(changes, |c| Value::from(c.clone())).0);
                }
                if let Some(fix) = fix {
                    let action = if fix.accept { "accept" } else { "reject" };
                    out.insert(
                        "fix".into(),
                        json!({"tool": "word_changes", "calls": [{"action": action, "ids": fix.ids}]}),
                    );
                }
            }
        }
        GridMismatch::Ragged {
            table,
            grid,
            rows,
            after_accept,
        } => {
            let (shown, left) = capped(rows, |r| json!({"row": r.row, "covers": [r.covers.0, r.covers.1]}));
            out.insert("table".into(), Value::from(table.clone()));
            out.insert("grid".into(), Value::from(*grid));
            out.insert("rows".into(), shown);
            out.insert("drawn".into(), Value::from("ragged"));
            out.insert("after_accept".into(), Value::Bool(*after_accept));
            if left > 0 {
                out.insert("rows_left_out".into(), Value::from(left));
            }
        }
    }
    Value::Object(out)
}

/// An `empty_paragraphs` entry: where, the paragraphs' positions and ids, and the
/// `word_delete` calls that remove them.
#[must_use]
pub fn empty_run_json(run: &EmptyRun) -> Value {
    let calls: Vec<Value> = run.targets.iter().map(|t| json!({"target": t})).collect();
    json!({
        "kind": "empty_paragraphs",
        "at": run.at,
        "paragraphs": run.paragraphs,
        "ids": run.ids,
        "fix": {"tool": "word_delete", "calls": calls},
    })
}

/// A `numbering_skips_level` entry (`shows` only where known).
#[must_use]
pub fn skipped_level_json(level: &SkippedLevel) -> Value {
    let mut out = Map::new();
    out.insert("para".into(), Value::from(level.para.clone()));
    out.insert("style".into(), level.style.clone().map_or(Value::Null, Value::from));
    out.insert("level".into(), Value::from(level.level));
    out.insert("list".into(), Value::from(level.list));
    out.insert("unused_levels".into(), Value::from(level.unused_levels.clone()));
    out.insert("label".into(), level.label.clone().map_or(Value::Null, Value::from));
    if let Some(shows) = &level.shows {
        out.insert("shows".into(), Value::from(shows.clone()));
    }
    out.insert("paragraphs".into(), Value::from(level.paragraphs));
    Value::Object(out)
}

/// An `accept_loses_text` entry, with the reject that keeps the cell: of its change where
/// it is named, else of every change within the cell.
#[must_use]
pub fn lost_cell_json(cell: &LostCell) -> Value {
    let call = match &cell.change {
        Some(change) => json!({"action": "reject", "ids": [change]}),
        None => json!({"action": "reject", "within": cell.cell}),
    };
    json!({
        "cell": cell.cell,
        "text": cell.text,
        "change": cell.change,
        "author": cell.author,
        "fix": {"tool": "word_changes", "calls": [call]},
    })
}

fn format_value(value: &FormatValue) -> Value {
    match value {
        FormatValue::Unset => Value::Null,
        FormatValue::Flag(on) => Value::Bool(*on),
        FormatValue::Number(n) => Value::from(*n),
        FormatValue::Name(name) => Value::from(name.clone()),
        FormatValue::Ruled(rule, n) => json!([rule, n]),
    }
}

/// A `direct_formatting` entry: the paragraph, its style, the direct values and its styles',
/// the run properties on only some of its text (`in_part`, where any) and the `word_format`
/// call that clears the others (where any).
#[must_use]
pub fn direct_formatting_json(entry: &DirectFormatting) -> Value {
    let mut out = Map::new();
    out.insert("para".into(), Value::from(entry.para.clone()));
    out.insert("style".into(), entry.style.clone().map_or(Value::Null, Value::from));
    let props: Map<String, Value> = entry.props.iter().map(|(n, v, _)| ((*n).to_owned(), format_value(v))).collect();
    let base: Map<String, Value> = entry.props.iter().map(|(n, _, b)| ((*n).to_owned(), format_value(b))).collect();
    out.insert("props".into(), Value::Object(props));
    out.insert("style_has".into(), Value::Object(base));
    if !entry.in_part.is_empty() {
        out.insert("in_part".into(), Value::from(entry.in_part.clone()));
    }
    if !entry.props.is_empty() {
        let clear: Vec<&str> = entry.props.iter().map(|(n, _, _)| *n).collect();
        out.insert(
            "fix".into(),
            json!({"tool": "word_format", "calls": [{"targets": [entry.para], "clear": clear}]}),
        );
    }
    Value::Object(out)
}

/// A `styles_not_allowed` entry: the style, its kind, how many blocks are in it and the
/// first of them.
#[must_use]
pub fn style_use_json(entry: &StyleUse) -> Value {
    let blocks: Vec<&String> = entry.blocks.iter().take(STYLE_BLOCKS).collect();
    json!({"style": entry.style, "kind": entry.kind, "count": entry.blocks.len(), "blocks": blocks})
}
