//! Owns: the block ids and drawing numbers one version lost and gained against another
//! ([`IdsDelta`], [`ids_delta`]) -- what docx-agent's ledger notes between two states
//! (`Ledger.note`: gone, and back) -- found from the change itself: only the story parts
//! whose content differs are read, and in each only the subtrees the two versions do not
//! share (an element changed in place is descended into; the shared ones are passed by
//! pointer, as [`crate::touched`] passes them).
//!
//! An id moved within the change (a block cut here and put there) is in both versions'
//! changed subtrees, so it is neither gone nor come. A paraId repeated in a subtree the
//! change did not touch is not looked for: a repeat is re-issued by the first edit.
//!
//! Must not: keep anything, or say who made the change (the session's ledger does).

use crate::doc::WordDoc;
use crate::Result;
use ooxml_word_model::ids::{held_id, HeldId};
use ooxml_xml::{Element, Node};
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

/// What one version's ids are against the version before it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdsDelta {
    /// The ids the earlier version had and this one has not.
    pub gone: Vec<HeldId>,
    /// The ids this version has and the earlier one had not.
    pub came: Vec<HeldId>,
}

impl IdsDelta {
    /// Whether no id went or came.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.gone.is_empty() && self.came.is_empty()
    }
}

/// The ids `after` lost and gained against `before` (two versions of one document).
///
/// # Errors
/// When a story part of either version cannot be read.
pub fn ids_delta(before: &WordDoc, after: &WordDoc) -> Result<IdsDelta> {
    let mut parts: BTreeSet<String> = BTreeSet::new();
    for doc in [before, after] {
        parts.extend(doc.docx()?.parts.iter().map(|p| p.part.clone()));
    }
    let mut old: Vec<Arc<Element>> = Vec::new();
    let mut new: Vec<Arc<Element>> = Vec::new();
    let mut held = Held::default();
    for part in parts {
        let same = match (before.package().part(&part), after.package().part(&part)) {
            (Some(a), Some(b)) => a.same(b),
            (None, None) => true,
            _ => false,
        };
        if same {
            continue;
        }
        match (before.tree(&part)?, after.tree(&part)?) {
            (Some(a), Some(b)) => changed(a.root(), b.root(), &mut held, &mut old, &mut new),
            (Some(a), None) => old.push(Arc::clone(a.root())),
            (None, Some(b)) => new.push(Arc::clone(b.root())),
            (None, None) => {}
        }
    }
    for root in &old {
        visit(root, &mut |e| held.old.extend(held_id(e)));
    }
    for root in &new {
        visit(root, &mut |e| held.new.extend(held_id(e)));
    }
    let mut gone: Vec<HeldId> = held.old.difference(&held.new).copied().collect();
    let mut came: Vec<HeldId> = held.new.difference(&held.old).copied().collect();
    gone.sort_unstable();
    came.sort_unstable();
    Ok(IdsDelta { gone, came })
}

/// The ids of elements changed in place (their own, not their subtrees').
#[derive(Default)]
struct Held {
    old: HashSet<HeldId>,
    new: HashSet<HeldId>,
}

/// `a` and `b` (one place in two versions): an element changed in place is descended into
/// (its own id kept in `held`); every other differing stretch of children goes whole into
/// `old` and `new`.
fn changed(
    a: &Arc<Element>,
    b: &Arc<Element>,
    held: &mut Held,
    old: &mut Vec<Arc<Element>>,
    new: &mut Vec<Arc<Element>>,
) {
    if Arc::ptr_eq(a, b) {
        return;
    }
    held.old.extend(held_id(a));
    held.new.extend(held_id(b));
    for run in ooxml_xml::changed_runs(a.children(), b.children()) {
        let pair = (run.old.len() == 1 && run.new.len() == 1)
            .then(|| (a.children().get(run.old.start), b.children().get(run.new.start)))
            .and_then(|(x, y)| Some((x?.as_element()?, y?.as_element()?)))
            .filter(|(x, y)| x.name() == y.name());
        if let Some((x, y)) = pair {
            changed(x, y, held, old, new);
            continue;
        }
        old.extend(elements(a, run.old));
        new.extend(elements(b, run.new));
    }
}

/// The elements among `parent`'s children in `places`.
fn elements(parent: &Element, places: std::ops::Range<usize>) -> Vec<Arc<Element>> {
    places
        .filter_map(|k| parent.children().get(k).and_then(Node::as_element).cloned())
        .collect()
}

/// Call `f` on every element below `root`, itself included.
fn visit(root: &Element, f: &mut impl FnMut(&Element)) {
    f(root);
    let mut stack = vec![root.children().iter()];
    while let Some(top) = stack.last_mut() {
        match top.next() {
            Some(Node::Element(e)) => {
                f(e);
                stack.push(e.children().iter());
            }
            Some(_) => {}
            None => {
                stack.pop();
            }
        }
    }
}
