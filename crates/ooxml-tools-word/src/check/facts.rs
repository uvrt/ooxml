//! Owns: `check`'s facts read from the document alone as docx-agent words them in `layout`
//! (`empty_paragraphs`, `numbering_skips_level`, `accept_loses_text`): each entry's JSON,
//! with the calls that fix it.
//!
//! Must not: find the facts (`ooxml_word_edit::facts` does), filter or cap them.

use ooxml_word_edit::facts::{EmptyRun, LostCell, SkippedLevel};
use serde_json::{json, Map, Value};

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
