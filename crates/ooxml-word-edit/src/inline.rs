//! Owns: the inline primitives tracked text edits are built from, as docx-agent's
//! `edit/inline.py` has them: refusing a span that cuts a field or would delete a note
//! reference ([`check_span`]); splitting runs at a character ([`split_before`]), around a
//! span ([`isolate`]) or at an insertion point ([`position`]); removing what an edit
//! emptied ([`prune`]); and the relationship ids an element spells ([`relationship_ids`]);
//! and the untracked replacement of a span ([`replace_span`]) with the run it may need
//! ([`new_run`]).
//!
//! Must not: write revisions, or read outside the paragraph.

use crate::names::{R, W, XML};
use crate::sink::Keep;
use crate::text::diff::{coarse, Tag};
use crate::text::write::{anchor, apply, run_properties_for, set_spaced_text, spell, Side};
use crate::text::{atoms, runs, Atom, AtomKind, OBJECT};
use std::collections::BTreeSet;
use crate::work::{Id, Work};
use crate::EditError;

/// Refuse a span that cuts a field, or (`deleting`) would delete a note reference.
pub(crate) fn check_span(
    work: &Work,
    items: &[Atom],
    start: usize,
    end: usize,
    deleting: bool,
) -> Result<(), EditError> {
    if !(start <= end && end <= items.len()) {
        return Err(EditError::Span(format!(
            "offsets {start}:{end} are outside the paragraph's {} characters",
            items.len()
        )));
    }
    let span = items.get(start..end).unwrap_or_default();
    let mut owners = span.iter().map(|a| a.field);
    if let Some(first) = owners.next() {
        if owners.any(|o| o != first) {
            return Err(EditError::Span(
                "the span cuts a field, or runs from a field's result into other text".to_owned(),
            ));
        }
    }
    if deleting
        && span
            .iter()
            .any(|a| a.kind == AtomKind::Element && work.is_any(a.node, W, &["footnoteReference", "endnoteReference"]))
    {
        return Err(EditError::Span(
            "the span holds a note reference; deleting it would orphan the note".to_owned(),
        ));
    }
    Ok(())
}

/// Every relationship id an element spells, in `r:` attributes, in document order.
pub(crate) fn relationship_ids(work: &Work, node: Id) -> Vec<String> {
    let mut out = Vec::new();
    for element in work.descendants(node) {
        for attribute in work.attributes(element) {
            if attribute.name.namespace() == Some(R) {
                out.push(attribute.value.to_string());
            }
        }
    }
    out
}

/// Remove runs left with nothing but properties, and hyperlinks, insertions, moves-to and
/// smart tags left without a run; returns the relationship ids of the hyperlinks removed.
pub(crate) fn prune(work: &mut Work, paragraph: Id) -> Vec<String> {
    let mut released = Vec::new();
    let mut changed = true;
    while changed {
        changed = false;
        for node in work.iter(paragraph, W, &["r", "hyperlink", "ins", "moveTo", "smartTag"]) {
            let Some(parent) = work.parent(node) else { continue };
            if work.is(node, W, "r") || work.is_any(parent, W, &["rPr", "trPr", "numPr"]) {
                continue; // a paragraph mark's or row's revision record, not a container
            }
            let empty = work
                .children(node)
                .iter()
                .all(|c| !work.is_element(*c) || work.is(*c, W, "rPr"));
            if empty {
                if let Some(rid) = work.get(node, Some(R), "id").filter(|r| !r.is_empty()) {
                    released.push(rid.to_owned());
                }
                work.remove(node);
                changed = true;
            }
        }
    }
    released
}

/// Make `atom` the first content of its run: split its `w:t` at it, and the run before it.
pub(crate) fn split_before(work: &mut Work, atom: &Atom) -> bool {
    let Some(run) = atom.run else {
        return false; // an equation: its own boundary
    };
    let node = atom.node;
    let mut split = false;
    if atom.kind == AtomKind::Text && atom.offset > 0 {
        let text: Vec<char> = work.text(node).chars().collect();
        let (head, rest) = text.split_at(atom.offset.min(text.len()));
        let left = work.shallow_copy(node);
        work.remove_attr(left, Some(XML), "space");
        set_spaced_text(work, left, &head.iter().collect::<String>());
        work.add_previous(node, left);
        set_spaced_text(work, node, &rest.iter().collect::<String>());
        split = true;
    }
    let mut before = Vec::new();
    for &child in work.children(run) {
        if child == node {
            break;
        }
        if work.is_element(child) && !work.is(child, W, "rPr") {
            before.push(child);
        }
    }
    if !before.is_empty() {
        let left_run = work.shallow_copy(run);
        if let Some(properties) = work.find(run, W, "rPr") {
            let copy = work.deep_copy(properties);
            work.append(left_run, copy);
        }
        for child in before {
            work.append(left_run, child);
        }
        work.add_previous(run, left_run);
        split = true;
    }
    split
}

/// Split runs so that `[start, end)` is exactly the content of whole runs; those runs in
/// document order.
pub(crate) fn isolate(work: &mut Work, paragraph: Id, start: usize, end: usize) -> Result<Vec<Id>, EditError> {
    let mut items = atoms(work, paragraph);
    check_span(work, &items, start, end, false)?;
    if start == end {
        return Ok(Vec::new());
    }
    if let Some(atom) = items.get(end).copied() {
        split_before(work, &atom);
        items = atoms(work, paragraph);
    }
    if let Some(atom) = items.get(start).copied() {
        split_before(work, &atom);
    }
    items = atoms(work, paragraph);
    let mut out: Vec<Id> = Vec::new();
    for atom in items.get(start..end).unwrap_or_default() {
        if let Some(run) = atom.run {
            if out.last() != Some(&run) {
                out.push(run);
            }
        }
    }
    Ok(out)
}

/// Where an element inserted at character `offset` goes, `(parent, index)`, after
/// splitting the run there. At the end it follows the paragraph's last run; in a paragraph
/// without text, it goes after the paragraph's properties and any markers.
pub(crate) fn position(work: &mut Work, paragraph: Id, offset: usize) -> Result<(Id, usize), EditError> {
    let items = atoms(work, paragraph);
    if offset > items.len() {
        return Err(EditError::Span(format!(
            "offset {offset} is outside the paragraph's {} characters",
            items.len()
        )));
    }
    if let Some(atom) = items.get(offset).copied() {
        split_before(work, &atom);
        let items = atoms(work, paragraph);
        let Some(atom) = items.get(offset) else {
            return Ok((paragraph, work.children(paragraph).len()));
        };
        let mut anchor = atom.run.unwrap_or(atom.node);
        if let Some(field) = atom.field {
            let first_of_field = offset == 0 || items.get(offset - 1).map(|a| a.field) != Some(Some(field));
            if first_of_field {
                anchor = if work.is(field, W, "fldSimple") {
                    field
                } else {
                    work.parent(field).unwrap_or(anchor)
                };
            }
        }
        let parent = work.parent(anchor).unwrap_or(paragraph);
        return Ok((parent, work.index(anchor).unwrap_or(0)));
    }
    if let Some(mut last) = runs(work, paragraph).last().copied() {
        while let Some(parent) = work.parent(last).filter(|p| work.is(*p, W, "fldSimple")) {
            last = parent; // after a simple field, not inside its result
        }
        let parent = work.parent(last).unwrap_or(paragraph);
        return Ok((parent, work.index(last).map_or(0, |i| i + 1)));
    }
    Ok((paragraph, work.children(paragraph).len()))
}

/// Replace the current view's characters `[start, end)` with `new`, untracked (docx-agent's
/// `inline.replace_span`): with `Keep::First` the new text takes the first replaced
/// character's formatting; with `Keep::Characters` each surviving character keeps its own
/// (the in-place diff). Text inserted beside nothing but a field's result is a run of its
/// own, outside the field. Returns the relationship ids of the drawings and objects it
/// removed (to release).
pub(crate) fn replace_span(
    work: &mut Work,
    paragraph: Id,
    (start, end): (usize, usize),
    new: &str,
    keep: Keep,
) -> Result<Vec<String>, EditError> {
    let items = atoms(work, paragraph);
    check_span(work, &items, start, end, true)?;
    if new.contains(OBJECT) {
        return Err(EditError::Span("new text cannot create an object (U+FFFC)".to_owned()));
    }
    let old: Vec<char> = items.get(start..end).unwrap_or_default().iter().map(|a| a.ch).collect();
    let chars: Vec<char> = new.chars().collect();
    if old == chars {
        return Ok(Vec::new());
    }
    let mut deleted = BTreeSet::new();
    let mut inserts = Vec::new();
    if old.is_empty() {
        let (at, side) = anchor(&items, start, end);
        let beside_field = at
            .and_then(|k| items.get(k))
            .is_some_and(|a| matches!(side, Side::Before | Side::After) && !a.plain());
        if beside_field {
            let children = spell(work, new, None);
            new_run(work, paragraph, start, &children)?;
            return Ok(Vec::new());
        }
        inserts.push((at, side, new.to_owned()));
    } else if keep == Keep::First {
        deleted.extend(start..end);
        if !new.is_empty() {
            let (at, side) = anchor(&items, start, end);
            inserts.push((at, side, new.to_owned()));
        }
    } else {
        for op in coarse(&old, &chars) {
            if op.tag == Tag::Equal {
                continue;
            }
            deleted.extend(start + op.i1..start + op.i2);
            if op.j2 > op.j1 {
                let (at, side) = anchor(&items, start + op.i1, start + op.i2);
                inserts.push((at, side, chars.get(op.j1..op.j2).unwrap_or_default().iter().collect()));
            }
        }
    }
    let mut released: Vec<String> = deleted
        .iter()
        .filter_map(|k| items.get(*k))
        .filter(|a| a.kind == AtomKind::Element)
        .flat_map(|a| relationship_ids(work, a.node))
        .collect();
    apply(work, paragraph, &items, &deleted, &inserts);
    released.extend(prune(work, paragraph));
    Ok(released)
}

/// A run holding `children`, put at `offset` with the formatting new text there takes (the
/// character before it, else the one after, else the paragraph mark's): docx-agent's
/// `new_run`.
pub(crate) fn new_run(work: &mut Work, paragraph: Id, offset: usize, children: &[Id]) -> Result<Id, EditError> {
    let items = atoms(work, paragraph);
    let reference = offset
        .checked_sub(1)
        .and_then(|k| items.get(k))
        .and_then(|a| a.run)
        .or_else(|| items.get(offset).and_then(|a| a.run));
    let properties = run_properties_for(work, paragraph, reference);
    let run = work.new_element(work.element_name(paragraph, W, "r", "w"));
    if let Some(properties) = properties {
        work.append(run, properties);
    }
    for child in children {
        work.append(run, *child);
    }
    let (parent, index) = position(work, paragraph, offset)?;
    work.insert(parent, index, run);
    Ok(run)
}
