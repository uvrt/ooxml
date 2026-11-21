//! Owns: `check`'s facts read from the document alone as docx-agent words them in `layout`
//! (`empty_paragraphs`, `numbering_skips_level`, `table_grid_mismatch`, `accept_loses_text`,
//! `direct_formatting`, `styles_not_allowed`, `accept_loses_original`, `row_text_format`,
//! `punctuation_spacing`, `text_language`): each entry's JSON, with the calls that fix it, an
//! entry's own lists capped as docx-agent caps them.
//!
//! Must not: find the facts (`ooxml_word_edit::facts` does), filter or cap them.

use crate::describe::short;
use ooxml_word_edit::facts::{
    DirectFormatting, EmptyRun, FormatValue, GridMismatch, LostCell, LostOriginal, Mistagged, SetLanguage,
    SkippedLevel, SpacingMark, SpacingProblem, SpotKind, StyleUse, TextLanguage,
};
use ooxml_word_edit::{LookField, RowUnlike, TextLook};
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
    let props: Map<String, Value> = entry
        .props
        .iter()
        .map(|(n, v, _)| ((*n).to_owned(), format_value(v)))
        .collect();
    let base: Map<String, Value> = entry
        .props
        .iter()
        .map(|(n, _, b)| ((*n).to_owned(), format_value(b)))
        .collect();
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

/// A look's `fields` as a `row_text_format` entry's `has` or `around` says them (`font`,
/// `sz`, `color`), or as `word_format` takes them (`font`, `size`, `color`).
fn look_json(look: &TextLook, fields: &[LookField], arguments: bool) -> Map<String, Value> {
    let mut out = Map::new();
    for field in fields {
        let value = match field {
            LookField::Font => look.font.clone().map_or(Value::Null, Value::from),
            LookField::Size => Value::from(look.size),
            LookField::Color => Value::from(look.color.clone()),
        };
        let key = if arguments { field.argument() } else { field.key() };
        out.insert(key.into(), value);
    }
    out
}

/// A `row_text_format` entry: a written row unlike the rows around it, its cells (capped)
/// with what they have and what the rows around share, and the `word_format` calls that
/// make them match.
#[must_use]
pub fn row_unlike_json(entry: &RowUnlike) -> Value {
    let (cells, _) = capped(&entry.cells, |c| {
        json!({"cell": c.cell, "has": look_json(&c.has, &c.fields, false),
            "around": look_json(&c.around, &c.fields, false)})
    });
    let calls: Vec<Value> = entry
        .fix
        .iter()
        .map(|call| {
            let mut one = Map::new();
            one.insert("targets".into(), Value::from(call.targets.clone()));
            one.extend(look_json(&call.look, &call.fields, true));
            Value::Object(one)
        })
        .collect();
    json!({"table": entry.table, "row": entry.row, "cells": cells,
        "fix": {"tool": "word_format", "calls": calls}})
}

/// The marks a `punctuation_spacing` entry shows, and fixes, at most (docx-agent's
/// `MARKS_PER_PARAGRAPH`).
const MARKS_PER_PARAGRAPH: usize = 10;

/// A `punctuation_spacing` entry: the paragraph's spacing problems with the `replace_text`
/// calls that fix them, or its spaces a language that wants none has (`language_mismatch`)
/// with the `set_language` that retags it.
#[must_use]
pub fn spacing_problem_json(problem: &SpacingProblem) -> Value {
    let mut out = Map::new();
    let (para, language, marks) = match problem {
        SpacingProblem::Spacing(s) => (&s.para, &s.language, &s.marks),
        SpacingProblem::LanguageMismatch(m) => (&m.para, &m.language, &m.marks),
    };
    out.insert("para".into(), Value::from(para.clone()));
    if matches!(problem, SpacingProblem::LanguageMismatch(_)) {
        out.insert("kind".into(), Value::from("language_mismatch"));
    }
    out.insert("language".into(), language.clone().map_or(Value::Null, Value::from));
    let shown: Vec<Value> = marks
        .iter()
        .take(MARKS_PER_PARAGRAPH)
        .map(|m| mark_json(m, language.as_deref()))
        .collect();
    out.insert("marks".into(), Value::from(shown));
    if marks.len() > MARKS_PER_PARAGRAPH {
        out.insert("marks_left_out".into(), Value::from(marks.len() - MARKS_PER_PARAGRAPH));
    }
    let tagged = language.as_deref().unwrap_or("None");
    match problem {
        SpacingProblem::Spacing(spacing) => {
            let calls: Vec<Value> = spacing
                .calls(MARKS_PER_PARAGRAPH)
                .into_iter()
                .map(|c| json!({"range": c.range, "find": c.find, "replace": c.replace, "expect": "one"}))
                .collect();
            out.insert("fix".into(), json!({"tool": "replace_text", "calls": calls}));
        }
        SpacingProblem::LanguageMismatch(mismatch) => {
            out.insert("source".into(), Value::from(mismatch.source.clone()));
            match &mismatch.fix {
                None => {
                    let note = format!(
                        "French spacing in runs tagged {tagged} themselves: likely French text under the wrong tag; \
                         not a spacing fault if so"
                    );
                    out.insert("note".into(), Value::from(note));
                }
                Some(fix) => {
                    let note = format!(
                        "French spacing in text tagged {tagged}: likely French under the wrong language; the fix \
                         sets it (the text's own region if not {}: fr-BE, fr-CH, fr-CA...)",
                        fix.language
                    );
                    out.insert("note".into(), Value::from(note));
                    out.insert(
                        "fix".into(),
                        json!({"tool": "word_styles", "calls": [set_language_json(fix)]}),
                    );
                }
            }
        }
    }
    Value::Object(out)
}

/// A mark: where, what, and its language where it is not the entry's.
fn mark_json(mark: &SpacingMark, first: Option<&str>) -> Value {
    let kind = match mark.kind {
        SpotKind::Breakable => "breakable",
        SpotKind::Missing => "missing",
        SpotKind::SpaceBefore => "space_before",
        SpotKind::SpaceInside => "space_inside",
    };
    let mut out = Map::new();
    out.insert("at".into(), Value::from(mark.at));
    out.insert("kind".into(), Value::from(kind));
    if mark.language.as_deref() != first {
        out.insert(
            "language".into(),
            mark.language.clone().map_or(Value::Null, Value::from),
        );
    }
    Value::Object(out)
}

/// A `word_styles` `set_language` call (`name`: the style's alone).
fn set_language_json(fix: &SetLanguage) -> Value {
    let mut call = Map::new();
    call.insert("action".into(), Value::from("set_language"));
    call.insert("language".into(), Value::from(fix.language.clone()));
    if let Some(style) = &fix.style {
        call.insert("name".into(), Value::from(style.clone()));
    }
    Value::Object(call)
}

/// `text_language`: the document's (or the checked words') language next to its tag, with
/// the fix that retags it where they differ, and at most `cap` paragraphs in another
/// language than their tag (`mismatched_left_out` the rest); the counts of paragraphs not
/// judged where there are any.
#[must_use]
pub fn text_language_json(found: &TextLanguage, cap: usize) -> Value {
    let document = &found.document;
    let mut fact = Map::new();
    fact.insert("tag".into(), document.tag.clone().map_or(Value::Null, Value::from));
    fact.insert(
        "source".into(),
        Value::from(document.source.as_deref().unwrap_or("none")),
    );
    let mut out = Map::new();
    let key = if found.whole { "document" } else { "checked" };
    match &document.detected {
        None => {
            fact.insert("detected".into(), Value::Null);
            fact.insert("paragraphs".into(), Value::from(document.paragraphs));
            out.insert(key.into(), Value::Object(fact));
        }
        Some(dominant) => {
            fact.insert("detected".into(), Value::from(dominant.language));
            fact.insert("share".into(), Value::from(dominant.share));
            fact.insert("words".into(), Value::from(dominant.words));
            fact.insert("paragraphs".into(), Value::from(document.paragraphs));
            if let Some(retag) = &dominant.retag {
                fact.insert("mismatched".into(), Value::from(retag.mismatched));
                fact.insert(
                    "fix".into(),
                    json!({"tool": "word_styles", "calls": [{"action": "set_language", "language": retag.language}]}),
                );
                let note = format!(
                    "text mostly {}, proofed as {}; the fix sets the document's language (the text's own region if \
                     another)",
                    dominant.language,
                    document.tag.as_deref().unwrap_or("None")
                );
                fact.insert("note".into(), Value::from(note));
            }
            out.insert(key.into(), Value::Object(fact));
            if !found.mismatched.is_empty() {
                let shown: Vec<Value> = found.mismatched.iter().take(cap).map(mistagged_json).collect();
                out.insert("mismatched".into(), Value::from(shown));
            }
            if found.mismatched.len() > cap {
                out.insert("mismatched_left_out".into(), Value::from(found.mismatched.len() - cap));
            }
        }
    }
    if found.too_short > 0 {
        out.insert("too_short".into(), Value::from(found.too_short));
    }
    if found.unclear > 0 {
        out.insert("unclear".into(), Value::from(found.unclear));
    }
    Value::Object(out)
}

/// A paragraph (or a range of them) in another language than its tag.
fn mistagged_json(entry: &Mistagged) -> Value {
    let mut out = Map::new();
    match &entry.through {
        None => {
            out.insert("para".into(), Value::from(entry.para.clone()));
        }
        Some((last, paragraphs)) => {
            out.insert("range".into(), Value::from(format!("{}..{last}", entry.para)));
            out.insert("paragraphs".into(), Value::from(*paragraphs));
        }
    }
    out.insert("tag".into(), entry.tag.clone().map_or(Value::Null, Value::from));
    out.insert("source".into(), Value::from(entry.source.as_deref().unwrap_or("none")));
    out.insert("detected".into(), Value::from(entry.detected));
    out.insert("confidence".into(), Value::from(entry.confidence));
    out.insert("words".into(), Value::from(entry.words));
    Value::Object(out)
}
