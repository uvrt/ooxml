//! Owns: [`PropertyTracker`], formatting changes recorded as tracked changes, as
//! docx-agent's `revisions/track.py` `PropertyTracker` records them: the formatting of
//! every paragraph and run under some roots read before an edit that writes untracked,
//! then each change it made recorded -- a paragraph's in a `w:pPrChange`, its mark's and a
//! run's in a `w:rPrChange` ([`record_run_change`]) -- once it ends. A run the edit split
//! is known again by the key its pieces carry (docx-agent's temporary attribute).
//!
//! Must not: allocate revision ids other than through its [`Stamp`], change formatting,
//! or read outside its roots.

use crate::marks::{clean_properties, insert_in_order, record_paragraph_change};
use crate::names::W;
use crate::policy::Stamp;
use crate::validate::Orders;
use crate::work::{Id, Work};
use std::collections::HashMap;

/// A run's revision marks and change record: not formatting.
const RUN_RECORDS: &[&str] = &["ins", "del", "moveFrom", "moveTo", "rPrChange"];

/// The formatting of paragraphs and runs before an edit.
pub(crate) struct PropertyTracker {
    roots: Vec<Id>,
    /// A paragraph's properties and its mark's, by key.
    paragraphs: HashMap<u32, (Id, Id)>,
    /// A run's properties, by key.
    runs: HashMap<u32, Id>,
}

/// A detached copy of a `w:rPr` without its revision marks and change record; an empty one
/// for none.
pub(crate) fn clean_run_properties(work: &mut Work, properties: Option<Id>, owner: Id) -> Id {
    let out = work.new_element(work.element_name(owner, W, "rPr", "w"));
    let Some(properties) = properties else { return out };
    let kept: Vec<Id> = work
        .children(properties)
        .iter()
        .copied()
        .filter(|c| work.is_element(*c) && !work.is_any(*c, W, RUN_RECORDS))
        .collect();
    for child in kept {
        let copy = work.deep_copy(child);
        work.append(out, copy);
    }
    out
}

/// Give `owner` (a run, or a paragraph's `w:pPr` for its mark) a `w:rPrChange` holding
/// `old` (its run properties before the edit), unless they are the same -- or the change
/// it has records them, when a change back drops it (docx-agent's `record_run_change`).
pub(crate) fn record_run_change(work: &mut Work, owner: Id, old: Id, stamp: &mut Stamp, orders: &Orders) {
    let properties = work.find(owner, W, "rPr");
    let new = clean_run_properties(work, properties, owner);
    if let Some(existing) = properties.and_then(|p| work.find(p, W, "rPrChange")) {
        if let Some(original) = work.find(existing, W, "rPr") {
            let original = clean_run_properties(work, Some(original), owner);
            if work.same_xml(original, new) {
                work.remove(existing);
                if let Some(p) = properties.filter(|p| work.children(*p).is_empty()) {
                    work.remove(p);
                    if work.is(owner, W, "pPr") && work.children(owner).is_empty() {
                        work.remove(owner);
                    }
                }
            }
        }
        return;
    }
    if work.same_xml(old, new) {
        return;
    }
    let properties = properties.unwrap_or_else(|| {
        let p = work.new_element(work.element_name(owner, W, "rPr", "w"));
        insert_in_order(work, owner, p, orders);
        p
    });
    let record = stamp.make(work, "rPrChange");
    let copy = work.deep_copy(old);
    work.append(record, copy);
    insert_in_order(work, properties, record, orders);
}

/// The innermost revision container `node` is in, below its paragraph.
fn revision_owner(work: &Work, node: Id) -> Option<Id> {
    let mut parent = work.parent(node);
    while let Some(p) = parent {
        if work.is(p, W, "p") {
            return None;
        }
        if work.is_any(p, W, &["ins", "moveTo", "del", "moveFrom"]) {
            return Some(p);
        }
        parent = work.parent(p);
    }
    None
}

impl PropertyTracker {
    /// Read the formatting of every paragraph and run at or below `roots` (none inside
    /// another), each given a key.
    pub(crate) fn start(work: &mut Work, roots: &[Id]) -> PropertyTracker {
        let mut tracker = PropertyTracker {
            roots: roots.to_vec(),
            paragraphs: HashMap::new(),
            runs: HashMap::new(),
        };
        let mut counter = 0u32;
        for &root in roots {
            for node in work.iter(root, W, &["p", "r"]) {
                counter += 1;
                work.set_key(node, Some(counter));
                if work.is(node, W, "p") {
                    let properties = work.find(node, W, "pPr");
                    let mark = properties.and_then(|p| work.find(p, W, "rPr"));
                    let old = clean_properties(work, properties, node);
                    let old_mark = clean_run_properties(work, mark, node);
                    tracker.paragraphs.insert(counter, (old, old_mark));
                } else {
                    let properties = work.find(node, W, "rPr");
                    let old = clean_run_properties(work, properties, node);
                    tracker.runs.insert(counter, old);
                }
            }
        }
        tracker
    }

    /// Record what the edit changed: a paragraph's properties and its mark's; a run's,
    /// unless it is deleted or this author's own insertion (formatting one's own insertion
    /// is part of it). The keys are taken away.
    pub(crate) fn record(self, work: &mut Work, stamp: &mut Stamp, orders: &Orders) {
        for &root in &self.roots {
            for node in work.iter(root, W, &["p", "r"]) {
                let Some(key) = work.key(node) else { continue };
                work.set_key(node, None);
                if let Some(&(old, old_mark)) = self.paragraphs.get(&key).filter(|_| work.is(node, W, "p")) {
                    record_paragraph_change(work, node, old, stamp, orders);
                    record_mark(work, node, old_mark, stamp, orders);
                } else if let Some(&old) = self.runs.get(&key).filter(|_| work.is(node, W, "r")) {
                    let owner = revision_owner(work, node);
                    let inserted_by_other =
                        owner.is_some_and(|o| work.is_any(o, W, &["ins", "moveTo"]) && !stamp.same(work, o));
                    if owner.is_none() || inserted_by_other {
                        record_run_change(work, node, old, stamp, orders);
                    }
                }
            }
        }
    }
}

/// A paragraph mark's formatting change: a `w:pPr` the edit found empty is left alone;
/// one made here, or emptied, goes.
fn record_mark(work: &mut Work, paragraph: Id, old_mark: Id, stamp: &mut Stamp, orders: &Orders) {
    let properties = work.find(paragraph, W, "pPr");
    if properties.is_none() && work.children(old_mark).is_empty() {
        return;
    }
    let found = properties.map(|p| work.children(p).len());
    let properties = properties.unwrap_or_else(|| {
        let p = work.new_element(work.element_name(paragraph, W, "pPr", "w"));
        work.insert(paragraph, 0, p);
        p
    });
    record_run_change(work, properties, old_mark, stamp, orders);
    if work.parent(properties).is_some() && work.children(properties).is_empty() && found != Some(0) {
        work.remove(properties);
    }
}
