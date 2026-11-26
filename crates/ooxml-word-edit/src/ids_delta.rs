//! Owns: the block ids and drawing numbers one version lost and gained against another
//! ([`IdsDelta`], [`ids_delta`]) -- what docx-agent's ledger notes between two states
//! (`Ledger.note`: gone, and back). The block ids come from the version's change set
//! ([`VersionDelta`]): the paraIds of the blocks that went, came, were renamed or changed,
//! as each version writes them. The drawing numbers are found from the change itself: only
//! the story parts whose content differs are read, and in each only the subtrees the two
//! versions do not share (an element changed in place is descended into; the shared ones
//! are passed by pointer).
//!
//! An id moved within the change (a block cut here and put there) is in both versions, so
//! it is neither gone nor come. A paraId repeated in a block the change did not touch is not
//! looked for: a repeat is re-issued by the first edit.
//!
//! Must not: keep anything, or say who made the change (the session's ledger does).

use crate::doc::WordDoc;
use crate::Result;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::{held_id, hex_value, BlockDelta, BlockKey, HeldId, PartIds, StoryDelta, VersionDelta};
use ooxml_xml::{Element, Node};
use std::collections::HashSet;
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
    ids_delta_of(before, after, &after.delta_from(before))
}

/// [`ids_delta`], the block ids from `delta`, what `after` changed against `before`.
///
/// # Errors
/// When a story part of either version cannot be read.
pub fn ids_delta_of(before: &WordDoc, after: &WordDoc, delta: &VersionDelta) -> Result<IdsDelta> {
    let (old_docx, new_docx) = (before.docx()?, after.docx()?);
    let story = |docx: &Docx, part: &str| docx.parts.iter().find(|p| p.part == part).cloned();
    let mut old: Vec<Arc<Element>> = Vec::new();
    let mut new: Vec<Arc<Element>> = Vec::new();
    let mut held = Held::default();
    for change in &delta.stories {
        let (a, b) = (story(&old_docx, &change.part), story(&new_docx, &change.part));
        match (&change.delta, a, b) {
            (StoryDelta::Blocks(blocks), Some(a), Some(b)) => {
                block_ids(blocks, (&a.ids, &b.ids), &mut held);
                drawings_changed(a.root(), b.root(), &mut held);
            }
            (_, a, b) => {
                old.extend(a.map(|a| Arc::clone(a.root())));
                new.extend(b.map(|b| Arc::clone(b.root())));
            }
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

/// The paraIds (as values) of the blocks `delta` names, as each version writes them: the
/// earlier version's of those gone, renamed and changed, the later's of those come,
/// renamed and changed.
fn block_ids(delta: &BlockDelta, (old, new): (&PartIds, &PartIds), held: &mut Held) {
    let value = |ids: &PartIds, key: BlockKey| ids.raw_id_of(key).and_then(|raw| hex_value(&raw)).map(HeldId::Block);
    let renamed_from = delta.renamed_from();
    let old_key = |key: BlockKey| renamed_from.get(&key).copied().unwrap_or(key);
    let both = delta.changed.iter().chain(&delta.stamp_only).copied();
    let earlier = delta
        .gone
        .iter()
        .map(|(k, _)| *k)
        .chain(delta.renamed.iter().map(|(o, _)| *o))
        .chain(both.clone().map(old_key));
    held.old.extend(earlier.filter_map(|k| value(old, k)));
    let later = delta
        .came
        .iter()
        .map(|(k, _)| *k)
        .chain(delta.renamed.iter().map(|(_, n)| *n))
        .chain(both);
    held.new.extend(later.filter_map(|k| value(new, k)));
}

/// The ids of elements changed in place (their own, not their subtrees'): drawing numbers
/// only, the block ids coming from the change set.
#[derive(Default)]
struct Held {
    old: HashSet<HeldId>,
    new: HashSet<HeldId>,
}

/// The drawing numbers of `a` and `b` (one place in two versions) the change may have
/// moved: an element changed in place is descended into (its own number kept in `held`);
/// every other differing stretch of children is walked whole.
fn drawings_changed(a: &Arc<Element>, b: &Arc<Element>, held: &mut Held) {
    if Arc::ptr_eq(a, b) {
        return;
    }
    held.old.extend(drawing_id(a));
    held.new.extend(drawing_id(b));
    for run in ooxml_xml::changed_runs(a.children(), b.children()) {
        let pair = (run.old.len() == 1 && run.new.len() == 1)
            .then(|| (a.children().get(run.old.start), b.children().get(run.new.start)))
            .and_then(|(x, y)| Some((x?.as_element()?, y?.as_element()?)))
            .filter(|(x, y)| x.name() == y.name());
        if let Some((x, y)) = pair {
            drawings_changed(x, y, held);
            continue;
        }
        walk_drawings(elements(a, run.old), &mut held.old);
        walk_drawings(elements(b, run.new), &mut held.new);
    }
}

/// A drawing's number, where `e` holds one.
fn drawing_id(e: &Element) -> Option<HeldId> {
    held_id(e).filter(|id| matches!(id, HeldId::Drawing(_)))
}

/// Every drawing number below `roots`.
fn walk_drawings(roots: Vec<Arc<Element>>, into: &mut HashSet<HeldId>) {
    for root in &roots {
        visit(root, &mut |e| into.extend(drawing_id(e)));
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
