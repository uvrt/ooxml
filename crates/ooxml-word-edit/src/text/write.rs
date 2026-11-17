//! Owns: writing a paragraph's text untracked, as docx-agent's `edit/text.py` writes it --
//! an in-place diff ([`set_text`]): each new character takes the formatting of the one it
//! replaces, or of the one before it when inserted; surviving characters stay in their
//! `w:t`; only the `w:t` nodes whose text changes are rewritten and only runs the edit
//! empties are removed -- and the pieces the tracked rules build from: spelling text as
//! run content ([`spell`]), new runs ([`make_run`]), the formatting new text takes
//! ([`run_properties_for`]).
//!
//! Must not: write revisions (the tracked rules do), or read outside the paragraph.

use crate::names::{W, XML};
use crate::text::diff::{coarse, python_space, Tag};
use crate::text::{atoms, Atom, AtomKind, LINE_BREAK, NON_BREAKING_HYPHEN, OBJECT, PAGE_BREAK, SOFT_HYPHEN};
use crate::work::{Id, Work};
use crate::EditError;
use ooxml_xml::QName;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Where new text goes, relative to a character of the old text (or a run of its own).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    /// Into the `w:t` of the character, before it.
    Before,
    /// Into the `w:t` of the character, after it.
    After,
    /// A run of its own after the character's run.
    RunAfter,
    /// A run of its own before the character's run.
    RunBefore,
}

/// Refuse text a paragraph cannot hold.
pub(crate) fn check_text(text: &str) -> Result<(), EditError> {
    if text.contains(['\n', '\r']) {
        return Err(EditError::Text(
            "a paragraph's text cannot contain '\\n'; insert a paragraph, or use '\\v' for a line break".to_owned(),
        ));
    }
    Ok(())
}

/// Give `paragraph` the text `new` in the current view, keeping per-character formatting.
/// Returns whether anything changed. An object (U+FFFC) can be kept or deleted, not made.
pub(crate) fn set_text(work: &mut Work, paragraph: Id, new: &str) -> Result<bool, EditError> {
    check_text(new)?;
    let items = atoms(work, paragraph);
    let old: Vec<char> = items.iter().map(|a| a.ch).collect();
    let new: Vec<char> = new.chars().collect();
    if old == new {
        return Ok(false);
    }
    let mut deleted = std::collections::BTreeSet::new();
    let mut inserts = Vec::new();
    for op in coarse(&old, &new) {
        if op.tag == Tag::Equal {
            continue;
        }
        deleted.extend(op.i1..op.i2);
        let text: String = new.get(op.j1..op.j2).unwrap_or_default().iter().collect();
        if text.is_empty() {
            continue;
        }
        if text.contains(OBJECT) {
            return Err(EditError::Text(
                "new text cannot create an object (U+FFFC); only keep one".to_owned(),
            ));
        }
        let (at, side) = anchor(&items, op.i1, op.i2);
        inserts.push((at, side, text));
    }
    apply(work, paragraph, &items, &deleted, &inserts);
    Ok(true)
}

/// Where new text for the old stretch `[i1, i2)` goes: the first character it replaces
/// when that is text, else after the character before the stretch, else before the one
/// after it -- preferring text that is not a field's result -- else a run of its own.
pub(crate) fn anchor(items: &[Atom], i1: usize, i2: usize) -> (Option<usize>, Side) {
    let at = |k: usize| items.get(k);
    if let Some(k) = (i1..i2).find(|k| at(*k).is_some_and(Atom::plain)) {
        return (Some(k), Side::Before);
    }
    if let Some(k) = (i1..i2).find(|k| at(*k).is_some_and(|a| a.kind == AtomKind::Text)) {
        return (Some(k), Side::Before);
    }
    let before = i1.checked_sub(1);
    let after = (i2 < items.len()).then_some(i2);
    let tests: [fn(&Atom) -> bool; 2] = [Atom::plain, |a| a.kind == AtomKind::Text];
    for test in tests {
        if let Some(b) = before.filter(|b| at(*b).is_some_and(test)) {
            return (Some(b), Side::After);
        }
        if let Some(a) = after.filter(|a| at(*a).is_some_and(test)) {
            return (Some(a), Side::Before);
        }
    }
    match (before, after) {
        (Some(b), _) => (Some(b), Side::RunAfter),
        (None, Some(a)) => (Some(a), Side::RunBefore),
        (None, None) => (None, Side::RunAfter),
    }
}

/// docx-agent's `_apply`: rewrite each changed `w:t` from its characters, remove deleted
/// tabs, breaks and objects, give text with no text beside it a run of its own, and remove
/// the runs the edit emptied.
pub(crate) fn apply(
    work: &mut Work,
    paragraph: Id,
    items: &[Atom],
    deleted: &std::collections::BTreeSet<usize>,
    inserts: &[(Option<usize>, Side, String)],
) {
    let mut before: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    let mut after: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    let mut new_runs = Vec::new();
    for (index, side, text) in inserts {
        match (side, index) {
            (Side::Before, Some(k)) => before.entry(*k).or_default().push(text),
            (Side::After, Some(k)) => after.entry(*k).or_default().push(text),
            _ => new_runs.push((*index, *side, text.as_str())),
        }
    }
    let mut touched: Vec<Option<Id>> = Vec::new();
    // each w:t, in the order its first character comes
    let mut by_node: Vec<(Id, Vec<usize>)> = Vec::new();
    for (index, atom) in items.iter().enumerate() {
        if atom.kind != AtomKind::Text {
            continue;
        }
        match by_node.iter_mut().find(|(n, _)| *n == atom.node) {
            Some((_, indices)) => indices.push(index),
            None => by_node.push((atom.node, vec![index])),
        }
    }
    for (node, indices) in by_node {
        let changed = indices
            .iter()
            .any(|k| deleted.contains(k) || before.contains_key(k) || after.contains_key(k));
        if !changed {
            continue;
        }
        let mut text = String::new();
        for k in &indices {
            text.extend(before.get(k).into_iter().flatten().copied());
            if !deleted.contains(k) {
                text.extend(items.get(*k).map(|a| a.ch));
            }
            text.extend(after.get(k).into_iter().flatten().copied());
        }
        replace_text_node(work, node, &text);
        touched.push(indices.first().and_then(|k| items.get(*k)).and_then(|a| a.run));
    }
    for k in deleted {
        if let Some(atom) = items.get(*k).filter(|a| a.kind == AtomKind::Element) {
            work.remove(atom.node);
            if atom.run.is_some() {
                touched.push(atom.run);
            }
        }
    }
    for (index, side, text) in new_runs {
        insert_run(work, paragraph, index.and_then(|k| items.get(k)), side, text);
    }
    let mut seen = Vec::new();
    for run in touched.into_iter().flatten() {
        if seen.contains(&run) {
            continue;
        }
        seen.push(run);
        if work
            .children(run)
            .iter()
            .all(|c| !work.is_element(*c) || work.is(*c, W, "rPr"))
        {
            work.remove(run);
        }
    }
}

/// Replace a `w:t` with the elements that spell `text`, in the same run.
pub(crate) fn replace_text_node(work: &mut Work, node: Id, text: &str) {
    for element in spell(work, text, Some(node)) {
        work.add_previous(node, element);
    }
    work.remove(node);
}

/// The elements that spell `text`: `w:t` for plain stretches (with `template`'s attributes,
/// when given), `w:tab`, `w:br`... for the special characters.
pub(crate) fn spell(work: &mut Work, text: &str, template: Option<Id>) -> Vec<Id> {
    let mut out = Vec::new();
    let mut plain = String::new();
    for ch in text.chars() {
        let special = match ch {
            '\t' => Some(("tab", None)),
            LINE_BREAK => Some(("br", None)),
            PAGE_BREAK => Some(("br", Some("page"))),
            NON_BREAKING_HYPHEN => Some(("noBreakHyphen", None)),
            SOFT_HYPHEN => Some(("softHyphen", None)),
            _ => None,
        };
        let Some((local, kind)) = special else {
            plain.push(ch);
            continue;
        };
        if !plain.is_empty() {
            out.push(text_element(work, &plain, template));
            plain.clear();
        }
        let element = work.new_element(work.qname(W, local, "w"));
        if let Some(kind) = kind {
            let name = work.qname(W, "type", "w");
            work.set(element, name, kind);
        }
        out.push(element);
    }
    if !plain.is_empty() {
        out.push(text_element(work, &plain, template));
    }
    out
}

/// A `w:t` of `text`: `template`'s attributes, and `xml:space="preserve"` where the text
/// has white space at either end or two spaces in a row.
fn text_element(work: &mut Work, text: &str, template: Option<Id>) -> Id {
    let node = work.new_element(work.qname(W, "t", "w"));
    if let Some(template) = template {
        for attribute in work.attributes(template).to_vec() {
            work.set(node, attribute.name, &attribute.value);
        }
    }
    set_spaced_text(work, node, text);
    node
}

/// Set a `w:t`'s text, with `xml:space="preserve"` where the text has white space at
/// either end or two spaces in a row (one it has already stays).
pub(crate) fn set_spaced_text(work: &mut Work, node: Id, text: &str) {
    work.set_text(node, text);
    let stripped = text.trim_matches(python_space);
    if stripped.len() != text.len() || text.contains("  ") {
        work.set(node, Arc::new(QName::new(Some("xml"), "space", Some(XML))), "preserve");
    }
}

/// A copy of the run properties new text near `reference` (a run) takes: the run's own,
/// else the paragraph mark's without its revision marks (`None` when that leaves none).
pub(crate) fn run_properties_for(work: &mut Work, paragraph: Id, reference: Option<Id>) -> Option<Id> {
    if let Some(run) = reference {
        let properties = work.find(run, W, "rPr")?;
        return Some(work.deep_copy(properties));
    }
    let mark = work.find(paragraph, W, "pPr").and_then(|p| work.find(p, W, "rPr"))?;
    let copy = work.deep_copy(mark);
    for child in work.children(copy).to_vec() {
        if work.is_any(child, W, &["ins", "del", "moveFrom", "moveTo", "rPrChange"]) {
            work.remove(child);
        }
    }
    (!work.children(copy).is_empty()).then_some(copy)
}

/// A run of `text` with `properties` (detached).
pub(crate) fn make_run(work: &mut Work, text: &str, properties: Option<Id>) -> Id {
    let run = work.new_element(work.qname(W, "r", "w"));
    if let Some(properties) = properties {
        work.append(run, properties);
    }
    for element in spell(work, text, None) {
        work.append(run, element);
    }
    run
}

/// Text with no text beside it: a run of its own, formatted as the reference's run.
fn insert_run(work: &mut Work, paragraph: Id, reference: Option<&Atom>, side: Side, text: &str) {
    let source = reference.and_then(|a| a.run);
    let properties = run_properties_for(work, paragraph, source);
    let run = make_run(work, text, properties);
    let Some(reference) = reference else {
        work.append(paragraph, run);
        return;
    };
    let anchor = reference.run.unwrap_or(reference.node);
    if side == Side::RunAfter {
        work.add_next(anchor, run);
    } else {
        work.add_previous(anchor, run);
    }
}
