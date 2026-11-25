//! Owns: a paragraph's list properties written in its [`Work`] copy, as docx-agent's
//! `edit/numbering.py` writes them -- its own `w:numPr` given ([`set_numbering`]) or taken
//! away ([`clear_numbering`]), its `w:pStyle` set or removed; and a list level's fields
//! (`w:start`, `w:numFmt`, `w:lvlText`, `w:suff`, positions and tab stop) in the level's
//! copy ([`set_level`]).
//!
//! Must not: read the document or write but into the `Work` it is given.

use crate::marks::insert_in_order;
use crate::names::W;
use crate::units::Twips;
use crate::validate::Orders;
use crate::work::{Id, Work};
use ooxml_word_model::xmlread::py_int;

/// The child `w:{local}` of `parent`, made where the schema wants it when missing
/// (ooxml-edit's `append_in_order(parent, make(...))`).
fn child_or_new(work: &mut Work, parent: Id, local: &str, orders: &Orders) -> Id {
    if let Some(found) = work.find(parent, W, local) {
        return found;
    }
    let made = work.new_element(work.element_name(parent, W, local, "w"));
    insert_in_order(work, parent, made, orders);
    made
}

/// Give `paragraph` its own `w:numPr` naming `num_id` at `level` (docx-agent's
/// `set_numbering`).
pub(crate) fn set_numbering(work: &mut Work, paragraph: Id, (num_id, level): (i64, i64), orders: &Orders) {
    let properties = child_or_new(work, paragraph, "pPr", orders);
    let numbering = child_or_new(work, properties, "numPr", orders);
    let old: Vec<Id> = work
        .children(numbering)
        .iter()
        .copied()
        .filter(|c| work.is_any(*c, W, &["ilvl", "numId"]))
        .collect();
    for node in old {
        work.remove(node);
    }
    let val = work.qname(W, "val", "w");
    let ilvl = work.new_element(work.element_name(numbering, W, "ilvl", "w"));
    work.set(ilvl, val.clone(), &level.to_string());
    work.insert(numbering, 0, ilvl);
    let reference = work.new_element(work.element_name(numbering, W, "numId", "w"));
    work.set(reference, val, &num_id.to_string());
    work.add_next(ilvl, reference);
}

/// Take `paragraph`'s own `w:numPr` away, and its `w:pPr` if that leaves it empty
/// (docx-agent's `clear_numbering`).
pub(crate) fn clear_numbering(work: &mut Work, paragraph: Id) {
    let Some(properties) = work.find(paragraph, W, "pPr") else {
        return;
    };
    if let Some(numbering) = work.find(properties, W, "numPr") {
        work.remove(numbering);
        if work.children(properties).is_empty() {
            work.remove(properties);
        }
    }
}

/// The style `paragraph` names, if any.
pub(crate) fn style_of(work: &Work, paragraph: Id) -> Option<String> {
    let properties = work.find(paragraph, W, "pPr")?;
    let node = work.find(properties, W, "pStyle")?;
    work.get(node, Some(W), "val").map(str::to_owned)
}

/// Remove `paragraph`'s `w:pStyle`, and its `w:pPr` if that leaves it empty.
pub(crate) fn remove_style(work: &mut Work, paragraph: Id) {
    let Some(properties) = work.find(paragraph, W, "pPr") else {
        return;
    };
    if let Some(node) = work.find(properties, W, "pStyle") {
        work.remove(node);
        if work.children(properties).is_empty() {
            work.remove(properties);
        }
    }
}

/// What a list level's format changes (docx-agent's `set_list_format`'s fields; positions
/// in points).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LevelFormat {
    /// The level (default: the paragraph's own).
    pub level: Option<i64>,
    /// `w:numFmt`.
    pub format: Option<String>,
    /// `w:lvlText`.
    pub text: Option<String>,
    /// `w:start`.
    pub start: Option<i64>,
    /// The number's position.
    pub number_at: Option<Twips>,
    /// The text's position.
    pub text_at: Option<Twips>,
    /// What follows the number: `tab`, `space` or `nothing`.
    pub follow: Option<String>,
    /// The level's tab stop.
    pub tab_at: Option<Twips>,
}

/// Write `format`'s fields into the level `lvl` (docx-agent's `set_list_format` change).
pub(crate) fn set_level(work: &mut Work, lvl: Id, format: &LevelFormat, orders: &Orders) {
    let val = work.qname(W, "val", "w");
    let start = format.start.map(|s| s.to_string());
    for (local, value) in [("start", &start), ("numFmt", &format.format), ("lvlText", &format.text)] {
        if let Some(value) = value {
            let node = child_or_new(work, lvl, local, orders);
            work.set(node, val.clone(), value);
        }
    }
    if let Some(follow) = &format.follow {
        let suffix = work.find(lvl, W, "suff");
        if follow == "tab" {
            if let Some(suffix) = suffix {
                work.remove(suffix);
            }
        } else {
            let suffix = suffix.unwrap_or_else(|| child_or_new(work, lvl, "suff", orders));
            work.set(suffix, val, follow);
        }
    }
    if format.number_at.is_some() || format.text_at.is_some() || format.tab_at.is_some() {
        positions(work, lvl, format, orders);
    }
}

/// A level's number and text positions (`w:ind`) and its `num` tab stop (docx-agent's
/// `_level_positions`).
fn positions(work: &mut Work, lvl: Id, format: &LevelFormat, orders: &Orders) {
    let properties = child_or_new(work, lvl, "pPr", orders);
    if format.number_at.is_some() || format.text_at.is_some() {
        let indent = child_or_new(work, properties, "ind", orders);
        let read = |work: &Work, names: &[&str]| {
            names
                .iter()
                .find_map(|n| work.get(indent, Some(W), n).and_then(py_int))
                .unwrap_or(0)
        };
        let left = read(work, &["left", "start"]);
        let number = left - read(work, &["hanging"]) + read(work, &["firstLine"]);
        let left = format.text_at.map_or(left, |at| at.0);
        let number = format.number_at.map_or(number, |at| at.0);
        for name in [
            "start",
            "hanging",
            "firstLine",
            "hangingChars",
            "firstLineChars",
            "startChars",
            "leftChars",
        ] {
            work.remove_attr(indent, Some(W), name);
        }
        let name = |work: &Work, local: &str| work.qname(W, local, "w");
        let left_name = name(work, "left");
        work.set(indent, left_name, &left.to_string());
        if number < left {
            let hanging = name(work, "hanging");
            work.set(indent, hanging, &(left - number).to_string());
        } else {
            let first = name(work, "firstLine");
            work.set(indent, first, &(number - left).to_string());
        }
    }
    if let Some(tab_at) = format.tab_at {
        tab_stop(work, properties, tab_at.0, orders);
    }
}

/// The level's `num` tab stop at `position` twips, before the first stop right of it.
fn tab_stop(work: &mut Work, properties: Id, position: i64, orders: &Orders) {
    let tabs = child_or_new(work, properties, "tabs", orders);
    let old: Vec<Id> = work
        .children(tabs)
        .iter()
        .copied()
        .filter(|t| work.is(*t, W, "tab") && work.get(*t, Some(W), "val") == Some("num"))
        .collect();
    for tab in old {
        work.remove(tab);
    }
    let tab = work.new_element(work.element_name(tabs, W, "tab", "w"));
    let val = work.qname(W, "val", "w");
    work.set(tab, val, "num");
    let pos = work.qname(W, "pos", "w");
    work.set(tab, pos, &position.to_string());
    let after = work
        .children(tabs)
        .iter()
        .copied()
        .find(|t| work.is(*t, W, "tab") && work.get(*t, Some(W), "pos").and_then(py_int).unwrap_or(0) > position);
    match after {
        Some(after) => work.add_previous(after, tab),
        None => work.append(tabs, tab),
    }
}
