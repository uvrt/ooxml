//! Owns: a paragraph's mark and properties in tracking mode, as docx-agent's
//! `revisions/track.py` writes them (measured on Word there): the mark recorded as
//! inserted or deleted ([`mark`], [`delete_mark`] -- one's own insertion removed outright,
//! the paragraph's content going to the start of the next), the properties a paragraph
//! takes recorded in a `w:pPrChange` ([`take_properties`], [`record_paragraph_change`]),
//! and children put where their parent's schema sequence wants them ([`insert_in_order`],
//! ooxml-edit's).
//!
//! Must not: allocate revision ids other than through its [`Stamp`], or read outside the
//! paragraphs it is given.

use crate::names::W;
use crate::policy::Stamp;
use crate::validate::Orders;
use crate::work::{Id, Work};

/// The paragraph mark's revision kinds.
const MARKS: &[&str] = &["ins", "del", "moveFrom", "moveTo"];

/// An element's name as the order table names it.
fn prefixed(work: &Work, id: Id, orders: &Orders) -> Option<String> {
    let name = work.name(id)?;
    Some(orders.prefixed_name(name.namespace(), name.local()))
}

/// Put `child` where `parent`'s schema sequence wants it (appended first when it is not
/// `parent`'s; a child the sequence does not name, or a parent without one, stays last).
pub(crate) fn insert_in_order(work: &mut Work, parent: Id, child: Id, orders: &Orders) {
    if work.parent(child) != Some(parent) {
        work.append(parent, child);
    }
    let Some(ranks) = prefixed(work, parent, orders).and_then(|p| orders.ranks(&p)) else {
        return;
    };
    let Some(rank) = prefixed(work, child, orders).and_then(|c| ranks.get(&c).copied()) else {
        return;
    };
    let unknown = ranks.len() + 1;
    let siblings: Vec<Id> = work.children(parent).to_vec();
    for sibling in siblings {
        if sibling == child || !work.is_element(sibling) {
            continue;
        }
        let theirs = prefixed(work, sibling, orders)
            .and_then(|s| ranks.get(&s).copied())
            .unwrap_or(unknown);
        if theirs > rank {
            work.add_previous(sibling, child);
            return;
        }
    }
    work.append(parent, child);
}

/// The paragraph mark's own revision record, if it has one.
pub(crate) fn mark_record(work: &Work, paragraph: Id) -> Option<Id> {
    let properties = work.find(paragraph, W, "pPr")?;
    let run = work.find(properties, W, "rPr")?;
    work.children(run).iter().copied().find(|c| work.is_any(*c, W, MARKS))
}

/// Record the paragraph's mark as `kind` (`ins`, `del`).
pub(crate) fn mark(work: &mut Work, paragraph: Id, kind: &str, stamp: &mut Stamp, orders: &Orders) -> Id {
    let properties = match work.find(paragraph, W, "pPr") {
        Some(p) => p,
        None => {
            let p = work.new_element(work.element_name(paragraph, W, "pPr", "w"));
            work.insert(paragraph, 0, p);
            p
        }
    };
    let run = match work.find(properties, W, "rPr") {
        Some(r) => r,
        None => {
            let r = work.new_element(work.element_name(properties, W, "rPr", "w"));
            insert_in_order(work, properties, r, orders);
            r
        }
    };
    let record = stamp.make(work, kind);
    insert_in_order(work, run, record, orders);
    record
}

/// The paragraph right after this one in its container, or `None` (a table, the end).
pub(crate) fn next_paragraph(work: &Work, paragraph: Id) -> Option<Id> {
    let mut at = work.next(paragraph);
    while let Some(node) = at {
        if work.is(node, W, "p") {
            return Some(node);
        }
        if work.is_any(node, W, &["tbl", "sdt", "customXml", "sectPr"]) {
            return None;
        }
        at = work.next(node);
    }
    None
}

/// Record a paragraph's mark as deleted -- or, when it is one's own insertion, take it
/// out: its content goes to the start of the next paragraph, which is returned.
pub(crate) fn delete_mark(work: &mut Work, paragraph: Id, stamp: &mut Stamp, orders: &Orders) -> Option<Id> {
    let record = mark_record(work, paragraph);
    let own_insertion = record.is_some_and(|r| work.is(r, W, "ins") && stamp.same(work, r));
    let Some(record) = record.filter(|_| own_insertion) else {
        if record.is_none_or(|r| work.is(r, W, "ins")) {
            mark(work, paragraph, "del", stamp, orders);
        }
        return None;
    };
    let following = next_paragraph(work, paragraph)?;
    let run = work.parent(record);
    work.remove(record);
    if let Some(run) = run.filter(|r| work.children(*r).is_empty()) {
        let properties = work.parent(run);
        work.remove(run);
        if let Some(properties) = properties.filter(|p| work.children(*p).is_empty()) {
            work.remove(properties);
        }
    }
    let from = usize::from(work.find(following, W, "pPr").is_some());
    let moving: Vec<Id> = work
        .children(paragraph)
        .iter()
        .copied()
        .filter(|c| !work.is(*c, W, "pPr"))
        .collect();
    for (at, child) in (from..).zip(moving) {
        work.insert(following, at, child);
    }
    work.remove(paragraph);
    Some(following)
}

/// A detached copy of a `w:pPr` without what is not formatting (its mark properties,
/// section break and change record); an empty one for none.
pub(crate) fn clean_properties(work: &mut Work, properties: Option<Id>, owner: Id) -> Id {
    let out = work.new_element(work.element_name(owner, W, "pPr", "w"));
    let Some(properties) = properties else { return out };
    let kept: Vec<Id> = work
        .children(properties)
        .iter()
        .copied()
        .filter(|c| work.is_element(*c) && !work.is_any(*c, W, &["rPr", "sectPr", "pPrChange"]))
        .collect();
    for child in kept {
        let copy = work.deep_copy(child);
        work.append(out, copy);
    }
    out
}

/// Give `paragraph` a `w:pPrChange` holding `old` (its properties before the edit), unless
/// they are the same -- or the change it has records them, when a change back drops it.
pub(crate) fn record_paragraph_change(work: &mut Work, paragraph: Id, old: Id, stamp: &mut Stamp, orders: &Orders) {
    let properties = work.find(paragraph, W, "pPr");
    let new = clean_properties(work, properties, paragraph);
    if let Some(existing) = properties.and_then(|p| work.find(p, W, "pPrChange")) {
        if let Some(original) = work.find(existing, W, "pPr") {
            let original = clean_properties(work, Some(original), paragraph);
            if work.same_xml(original, new) {
                work.remove(existing);
                if let Some(p) = properties.filter(|p| work.children(*p).is_empty()) {
                    work.remove(p);
                }
            }
        }
        return;
    }
    if work.same_xml(old, new) {
        return;
    }
    let properties = match properties {
        Some(p) => p,
        None => {
            let p = work.new_element(work.element_name(paragraph, W, "pPr", "w"));
            work.insert(paragraph, 0, p);
            p
        }
    };
    let record = stamp.make(work, "pPrChange");
    let copy = work.deep_copy(old);
    work.append(record, copy);
    insert_in_order(work, properties, record, orders);
}

/// Give `target` the paragraph properties `source` (a `w:pPr`), recording its old ones in
/// a `w:pPrChange`: how Word, tracking a deletion across a paragraph mark, makes accepting
/// it keep the first paragraph's properties.
pub(crate) fn take_properties(work: &mut Work, target: Id, source: Id, stamp: &mut Stamp, orders: &Orders) {
    let current = work.find(target, W, "pPr");
    let old = clean_properties(work, current, target);
    let new = clean_properties(work, Some(source), target);
    if work.same_xml(old, new) {
        return;
    }
    let properties = match current {
        Some(p) => p,
        None => {
            let p = work.new_element(work.element_name(target, W, "pPr", "w"));
            work.insert(target, 0, p);
            p
        }
    };
    let dropped: Vec<Id> = work
        .children(properties)
        .iter()
        .copied()
        .filter(|c| !work.is_any(*c, W, &["rPr", "sectPr", "pPrChange"]))
        .collect();
    for child in dropped {
        work.remove(child);
    }
    for child in work.children(new).to_vec() {
        let copy = work.deep_copy(child);
        insert_in_order(work, properties, copy, orders);
    }
    record_paragraph_change(work, target, old, stamp, orders);
}

/// Record a new paragraph's content as inserted: its children but the properties, in one
/// `w:ins` where the first of them stands (docx-agent's `insert_paragraph_content`).
pub(crate) fn insert_paragraph_content(work: &mut Work, paragraph: Id, stamp: &mut Stamp) {
    let content: Vec<Id> = work
        .children(paragraph)
        .iter()
        .copied()
        .filter(|c| work.is_element(*c) && !work.is(*c, W, "pPr"))
        .collect();
    let Some(&first) = content.first() else { return };
    let container = stamp.make(work, "ins");
    work.add_previous(first, container);
    for child in content {
        work.append(container, child);
    }
}

/// Word's typing form at a container's end (docx-agent's `pass_mark`): `previous` gets an
/// inserted mark, and the properties its old mark had -- before a change recorded on them,
/// which goes -- are returned (detached) for the new paragraph to record its own against.
fn pass_mark(work: &mut Work, previous: Id, stamp: &mut Stamp, orders: &Orders) -> Id {
    mark(work, previous, "ins", stamp, orders);
    let properties = work.find(previous, W, "pPr");
    let change = properties.and_then(|p| work.find(p, W, "pPrChange"));
    let recorded = change.and_then(|c| work.find(c, W, "pPr"));
    let old = clean_properties(work, recorded.or(properties), previous);
    if let Some(change) = change {
        work.remove(change);
    }
    old
}

/// Record `paragraph`, just put in place, as inserted (docx-agent's
/// `_track_new_paragraph`): its content in a `w:ins`; then, `last` in its container after a
/// paragraph without a mark record or a section break, as Word writes a paragraph typed at
/// the end of the one before (that one's mark the inserted one, the new paragraph's
/// properties a change from the old mark's); else its own mark inserted.
pub(crate) fn track_new_paragraph(work: &mut Work, paragraph: Id, last: bool, stamp: &mut Stamp, orders: &Orders) {
    insert_paragraph_content(work, paragraph, stamp);
    let mut previous = work.previous(paragraph);
    while let Some(node) = previous.filter(|n| !work.is_any(*n, W, &["p", "tbl"])) {
        previous = work.previous(node);
    }
    if let Some(previous) = previous.filter(|p| last && work.is(*p, W, "p") && mark_record(work, *p).is_none()) {
        let breaks = work.find(previous, W, "pPr").and_then(|p| work.find(p, W, "sectPr"));
        if breaks.is_none() {
            let old = pass_mark(work, previous, stamp, orders);
            record_paragraph_change(work, paragraph, old, stamp, orders);
            return;
        }
    }
    mark(work, paragraph, "ins", stamp, orders);
}
