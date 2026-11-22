//! Owns: the calls that give a copy's paragraphs their reference's list item or page
//! setting (docx-agent's `copy_settings.settings_fix`, the one place they are written; its
//! module's "Fixes"): `word_format` `like` the reference's paragraph where the reference is
//! in the same document and taking its look changes nothing else compared ([`like_takes`]);
//! otherwise `word_format` with the reference's value, or `clear` where the paragraph's
//! style gives it; `word_lists` `level`, `add`, `remove`, `format` (with the reference's
//! places where the indents differ), `restart` and `continue` (`join`: the previous copy's
//! list, in a stated run). `widow_control` has none (`word_follow_sample` gives it).
//!
//! Must not: compare settings (`super::differences` does).

use super::settings::{Settings, LIST_SETTINGS, RUNS_ON};
use crate::style::compare::{fix, Block, Side};
use crate::style::compare_paragraphs::PARAGRAPH;
use ooxml_base::pyfmt::round2;
use ooxml_word_model::docx::numbering::{membership, Lists};
use ooxml_word_model::docx::{find_path, Docx, StoryPart};
use ooxml_word_model::ids::W;
use ooxml_xml::Element;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// The settings `word_format` sets by the same name.
const FORMAT_FIELDS: [&str; 5] = [
    "indent_left",
    "indent_first",
    "keep_with_next",
    "keep_lines",
    "page_break_before",
];

/// The copy's document as its fixes read it.
pub(super) struct Fixing<'f, 'd> {
    /// Its formatting and styles.
    pub(super) side: &'f Side<'d>,
    /// Its reading.
    pub(super) docx: &'f Docx,
    /// Its body.
    pub(super) body: &'f StoryPart,
    /// Its numbering.
    pub(super) lists: &'f Lists,
    /// Whether the reference is in the same document.
    pub(super) same: bool,
}

/// One group of differences a fix is written for.
pub(super) struct Wanted<'w> {
    /// The setting.
    pub(super) name: &'static str,
    /// The copy's value.
    pub(super) value: &'w Value,
    /// The reference's.
    pub(super) reference: &'w Value,
    /// The copy's paragraphs.
    pub(super) paragraphs: &'w [String],
    /// Its settings.
    pub(super) sample_settings: &'w Settings,
    /// The first paragraph's.
    pub(super) own_settings: &'w Settings,
    /// Per paragraph, the sample paragraph of the same document it may look like.
    pub(super) likes: &'w [(String, Option<String>)],
    /// Per paragraph, the previous copy's item whose list it goes on from.
    pub(super) joins: &'w HashMap<String, String>,
    /// Whether a restart is offered (not inside a running list).
    pub(super) restart: bool,
}

/// The fix of `w` (`null`: no call makes it).
pub(super) fn settings_fix(f: &Fixing<'_, '_>, w: &Wanted<'_>) -> Value {
    let like_of = |p: &String| w.likes.iter().find(|(q, _)| q == p).and_then(|(_, like)| like.clone());
    if !w.likes.is_empty() && w.paragraphs.iter().all(|p| like_of(p).is_some()) {
        let mut by_sample: Vec<(String, Vec<String>)> = Vec::new();
        for p in w.paragraphs {
            let like = like_of(p).unwrap_or_default();
            match by_sample.iter_mut().find(|(l, _)| *l == like) {
                Some((_, targets)) => targets.push(p.clone()),
                None => by_sample.push((like, vec![p.clone()])),
            }
        }
        let calls = by_sample
            .into_iter()
            .map(|(like, targets)| json!({"targets": targets, "like": like}))
            .collect();
        return fix("word_format", calls);
    }
    if FORMAT_FIELDS.contains(&w.name) {
        let mut call = Map::new();
        call.insert("targets".into(), json!(w.paragraphs));
        if style_gives(f, w.name, w.reference, w.paragraphs) {
            call.insert("clear".into(), json!([w.name]));
        } else {
            call.insert(w.name.into(), w.reference.clone());
        }
        return fix("word_format", vec![Value::Object(call)]);
    }
    match w.name {
        "list_level" => list_level(w),
        "list_format" => list_format(f, w),
        "list_numbering" => list_numbering(f, w),
        _ => Value::Null,
    }
}

fn list_level(w: &Wanted<'_>) -> Value {
    let call = if w.reference.is_null() {
        json!({"targets": w.paragraphs, "action": "remove"})
    } else if w.value.is_null() {
        // A new list of Word's own at the reference's level; a check after it compares its
        // label with the reference's.
        let kind = if w.sample_settings.bullets() {
            "bullet"
        } else {
            "number"
        };
        json!({"targets": w.paragraphs, "action": "add", "kind": kind, "level": w.reference})
    } else {
        json!({"targets": w.paragraphs, "action": "level", "level": w.reference})
    };
    fix("word_lists", vec![call])
}

fn list_format(f: &Fixing<'_, '_>, w: &Wanted<'_>) -> Value {
    let reference = w.reference.as_str().unwrap_or("");
    let (number_format, label) = reference.split_once(' ').unwrap_or((reference, ""));
    let (own, sample) = (w.own_settings, w.sample_settings);
    let place = (own.indent_left, own.indent_first) != (sample.indent_left, sample.indent_first);
    let calls: Vec<Value> = first_per_list(f, w.paragraphs)
        .into_iter()
        .map(|(p, level)| {
            let mut call = Map::new();
            call.insert("targets".into(), json!([p]));
            call.insert("action".into(), json!("format"));
            call.insert("level".into(), json!(level));
            call.insert("number_format".into(), json!(number_format));
            call.insert("label".into(), json!(label));
            if place {
                // The label and text where the reference has them ("Define new multilevel list").
                call.insert("text_at".into(), json!(sample.indent_left));
                call.insert(
                    "number_at".into(),
                    json!(round2(sample.indent_left + sample.indent_first)),
                );
            }
            Value::Object(call)
        })
        .collect();
    if calls.is_empty() {
        Value::Null
    } else {
        fix("word_lists", calls)
    }
}

fn list_numbering(f: &Fixing<'_, '_>, w: &Wanted<'_>) -> Value {
    let reference = w.reference.as_str().unwrap_or("");
    let calls: Vec<Value> = if reference == RUNS_ON {
        w.paragraphs
            .iter()
            .filter_map(|p| {
                w.joins
                    .get(p)
                    .map(|join| json!({"targets": [p], "action": "continue", "join": join}))
            })
            .collect()
    } else if let Some(start) = reference.strip_prefix("starts at ") {
        let Ok(start) = start.trim().parse::<i64>() else {
            return Value::Null;
        };
        if !w.restart {
            return Value::Null;
        }
        return fix(
            "word_lists",
            w.paragraphs
                .iter()
                .map(|p| json!({"targets": [p], "action": "restart", "start": start}))
                .collect(),
        );
    } else {
        w.paragraphs
            .iter()
            .filter(|p| can_continue(f, p))
            .map(|p| json!({"targets": [p], "action": "continue"}))
            .collect()
    };
    if !calls.is_empty() && calls.len() == w.paragraphs.len() {
        fix("word_lists", calls)
    } else {
        Value::Null
    }
}

/// Whether `word_format` `like` the reference gives `paragraph` the reference's `name` and
/// changes nothing else this comparison reads (`like` takes the sample's style, list,
/// direct indents and page settings whole; the pair's styles are alike here): a page
/// setting or an indent where the two state the same list (or none); the list level or
/// label where the reference draws bullets, or one of the two is in no list; where its
/// numbering starts, never (`like` continues the sample's list).
pub(super) fn like_takes(
    name: &str,
    paragraph: &Element,
    reference: &Element,
    own: &Settings,
    sample: &Settings,
) -> bool {
    if LIST_SETTINGS.contains(&name) {
        if name == "list_numbering" {
            return false;
        }
        return own.list_level.is_none() || sample.list_level.is_none() || sample.bullets();
    }
    numbering_stated(paragraph) == numbering_stated(reference)
}

/// A paragraph's own `w:numPr` as `like` compares it: its level and instance.
fn numbering_stated(paragraph: &Element) -> Option<(Option<String>, Option<String>)> {
    let found = find_path(paragraph, &["pPr", "numPr"])?;
    let val = |local: &str| {
        found
            .find(Some(W), local)
            .and_then(|e| e.attribute(Some(W), "val"))
            .map(str::to_owned)
    };
    Some((val("ilvl"), val("numId")))
}

/// Whether every paragraph has `name` of its own and its style (and list) alone gives the
/// reference's value: clearing it is the fix, the template's value kept (rule 6).
fn style_gives(f: &Fixing<'_, '_>, name: &str, reference: &Value, paragraphs: &[String]) -> bool {
    let Some((_, tag, attributes)) = PARAGRAPH.iter().find(|(n, _, _)| *n == name) else {
        return false;
    };
    paragraphs.iter().all(|identifier| {
        let Ok(block) = Block::of(f.docx, identifier) else {
            return false;
        };
        let Some(node) = find_path(&block.element, &["pPr", tag]) else {
            return false;
        };
        if !attributes.is_empty() && !attributes.iter().any(|a| node.attribute(Some(W), a).is_some()) {
            return false;
        }
        let cell = f.side.cell(block.part, &block.element, &block.path);
        let applied = f.side.formatting().paragraph_styled(&block.element, cell.as_ref());
        let own = match name {
            "indent_left" => json!(round2(applied.indent_left)),
            "indent_first" => json!(round2(applied.first_line - applied.hanging)),
            "keep_with_next" => json!(applied.keep_with_next),
            "keep_lines" => json!(applied.keep_together),
            "page_break_before" => json!(applied.page_break_before),
            _ => Value::Null,
        };
        own == *reference
    })
}

/// The first of `paragraphs` in each list and level, with its level: one format call per
/// list.
fn first_per_list(f: &Fixing<'_, '_>, paragraphs: &[String]) -> Vec<(String, i64)> {
    let mut seen: Vec<(i64, i64)> = Vec::new();
    let mut out = Vec::new();
    for identifier in paragraphs {
        let Ok(block) = Block::of(f.docx, identifier) else {
            continue;
        };
        let Some(found) = membership(f.docx, f.lists, &block.element) else {
            continue;
        };
        if seen.contains(&(found.num_id, found.level)) {
            continue;
        }
        seen.push((found.num_id, found.level));
        out.push((identifier.clone(), found.level));
    }
    out
}

/// Whether `word_lists` `continue` finds a list to continue for the paragraph: an earlier
/// paragraph in another list over the same definition.
fn can_continue(f: &Fixing<'_, '_>, identifier: &str) -> bool {
    let Ok(block) = Block::of(f.docx, identifier) else {
        return false;
    };
    let Some(found) = membership(f.docx, f.lists, &block.element) else {
        return false;
    };
    let mine = f.lists.canonical(found.abstract_id);
    for other in f.body.ids.paragraphs().iter() {
        if other.id == block.id {
            break;
        }
        if let Some(listed) = membership(f.docx, f.lists, &other.element) {
            if listed.num_id != found.num_id && f.lists.canonical(listed.abstract_id) == mine {
                return true;
            }
        }
    }
    false
}
