//! Owns: one copy's entry in `word_follow_sample`'s reply (docx-agent's `follow._report`):
//! what changed, and each difference left (as `word_compare_blocks` finds it after
//! following, less the copy's own row and line counts) with why it was not followed and its
//! kind -- `own` (the copy's text, lines and lists, kept by design), `changed_by_copy` (a
//! style this document lacks and Word has no built-in one by its name) or `structure`
//! (columns, merges, the kind of block) -- capped per copy, the copy's own counted first.
//!
//! Must not: compare or change anything.

use ooxml_word_edit::CopyFollowed;
use serde_json::{json, Map, Value};

/// Differences reported per copy at most; `more` counts the rest.
const PER_COPY: usize = 20;
/// Differences reported in one reply at most, over every copy.
pub(super) const CAP: usize = 100;
/// Why a difference left in a cell merged otherwise than the sample's was not followed.
const WHY_OTHER: &str = "the cell is merged differently from the sample's";

/// Why each kind of difference left was not followed, and its kind.
fn why(property: &str) -> Option<(&'static str, &'static str)> {
    Some(match property {
        "kind" => ("the copy is not the same kind of block as the sample", "structure"),
        "columns" => (
            "the copy's columns hold its values: columns are not added or removed",
            "structure",
        ),
        "widths" => ("the copy has another number of columns than the sample", "structure"),
        "merges" => (
            "cells are not merged or split: it would move the copy's text",
            "structure",
        ),
        "missing_paragraph" => (
            "a line of the sample's text the copy's cell lacks: text is the copy's own",
            "own",
        ),
        "extra_paragraph" => (
            "a line no sample paragraph matches by text or style: it keeps its formatting",
            "own",
        ),
        "paragraph_style" | "table_style" => (
            "this document has no such style and Word has no built-in one by its name",
            "changed_by_copy",
        ),
        "list" => (
            "list membership is not followed: the sample's list is another list (word_lists)",
            "own",
        ),
        _ => return None,
    })
}

/// A copy's `own` differences taken out of `not_followed` and counted in `own`.
pub(super) fn own_counted(entry: &mut Map<String, Value>) {
    let Some(Value::Array(shown)) = entry.get_mut("not_followed") else {
        return;
    };
    let before = shown.len();
    shown.retain(|d| d.get("kind").and_then(Value::as_str) != Some("own"));
    let dropped = (before - shown.len()) as u64;
    if dropped > 0 {
        let own = entry.get("own").and_then(Value::as_u64).unwrap_or(0) + dropped;
        entry.insert("own".into(), json!(own));
    }
}

/// One difference left, as reported: its keys but `block`, with `why` and `kind`.
fn left_item(difference: &Value, copy: &CopyFollowed) -> Option<Value> {
    let property = difference.get("property").and_then(Value::as_str).unwrap_or("");
    if matches!(property, "rows" | "paragraphs") {
        return None; // a copy's own row count; a cell's line count, said by its lines
    }
    let mut item: Map<String, Value> = difference
        .as_object()
        .map(|d| {
            d.iter()
                .filter(|(k, _)| *k != "block")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();
    let mut found = why(property);
    if matches!(property, "paragraph_style" | "table_style") {
        let named = difference.get("reference").and_then(Value::as_str);
        if !named.is_some_and(|n| copy.missing.iter().any(|(m, _)| m == n)) {
            found = None;
        }
    }
    let (reason, kind) = found.unwrap_or((WHY_OTHER, "structure"));
    item.insert("why".into(), json!(reason));
    item.insert("kind".into(), json!(kind));
    Some(Value::Object(item))
}

/// One copy's entry: `copy`, `changed`, `asked` where it differs, what changed, and
/// `not_followed` (capped; `own` and `more` counting what is left out).
pub(super) fn copy_entry(copy: &CopyFollowed, left: &[Value]) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("copy".into(), json!(copy.id));
    out.insert("changed".into(), json!(copy.changed()));
    if copy.asked != copy.id {
        out.insert("asked".into(), json!(copy.asked));
    }
    if !copy.table.is_empty() {
        out.insert("table".into(), json!(copy.table));
    }
    if copy.widths {
        out.insert("widths".into(), json!(true));
    }
    for (name, count) in [
        ("rows", copy.rows),
        ("cells", copy.cells),
        ("paragraphs", copy.paragraphs),
        ("runs", copy.runs),
    ] {
        if count > 0 {
            out.insert(name.into(), json!(count));
        }
    }
    let mut shown: Vec<Value> = left.iter().filter_map(|d| left_item(d, copy)).collect();
    if !copy.lists.is_empty() {
        shown.push(json!({"property": "list", "paragraphs": copy.lists,
                          "why": why("list").map(|w| w.0), "kind": "own", "fix": null}));
    }
    let total = shown.len();
    out.insert("not_followed".into(), Value::Array(shown));
    if total > PER_COPY {
        own_counted(&mut out); // the copy's own become a count before anything is cut
    }
    if let Some(Value::Array(shown)) = out.get_mut("not_followed") {
        let more = shown.len().saturating_sub(PER_COPY);
        shown.truncate(PER_COPY);
        if more > 0 {
            out.insert("more".into(), json!(more));
        }
    }
    out
}
