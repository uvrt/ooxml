//! Owns: `check`'s facts read from the document alone as docx-agent words them in `layout`
//! (`empty_paragraphs`, `numbering_skips_level`, `table_grid_mismatch`, `accept_loses_text`,
//! `accept_loses_original`):
//! each entry's JSON, with the calls that fix it, an entry's own lists capped as docx-agent
//! caps them.
//!
//! Must not: find the facts (`ooxml_word_edit::facts` does), filter or cap them.

use crate::describe::short;
use ooxml_word_edit::facts::{EmptyRun, GridMismatch, LostCell, LostOriginal, SkippedLevel};
use serde_json::{json, Map, Value};

/// The most of a list inside one entry (docx-agent's `CAP`).
const CAP: usize = 20;

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

/// An `accept_loses_original` entry: the file's text one listed change of this session takes
/// out of the final view (per table, or the body's paragraphs), with the reject that keeps it.
#[must_use]
pub fn lost_original_json(entry: &LostOriginal) -> Value {
    let mut out = Map::new();
    out.insert("change".into(), entry.change.clone().map_or(Value::Null, Value::from));
    out.insert("author".into(), entry.author.clone().map_or(Value::Null, Value::from));
    match &entry.table {
        Some(table) => out.insert("table".into(), Value::from(table.clone())),
        None => out.insert("paras".into(), Value::from(entry.paras.clone())),
    };
    out.insert("cells".into(), Value::from(entry.cells));
    out.insert("paragraphs".into(), Value::from(entry.paragraphs));
    let sample: Vec<Value> = entry
        .sample
        .iter()
        .map(|(at, text)| json!({"at": at, "text": short(text, 60)}))
        .collect();
    out.insert("sample".into(), Value::from(sample));
    let call = match &entry.change {
        Some(change) => json!({"action": "reject", "ids": [change]}),
        None => json!({"action": "reject", "within": entry.within()}),
    };
    out.insert("fix".into(), json!({"tool": "word_changes", "calls": [call]}));
    Value::Object(out)
}
