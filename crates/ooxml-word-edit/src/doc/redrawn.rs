//! Owns: [`Redrawn`], what a transaction's earlier splices put in place as its changes are
//! applied: each element a splice with draws inserted, as applied, and where its draws wrote.
//! Replayed on a newer head than the one it was prepared on, a draw comes out as another
//! value (another writer drew the one the transaction did), so a later change of the same
//! transaction that replaces such an element whole -- a batch's second edit of a paragraph
//! its first edit wrote -- removes it as the transaction's working version had it, not as
//! it is now. [`Redrawn::patch`] names it as it is now, so the transaction replays instead
//! of being prepared again.
//!
//! Must not: live past one transaction's apply (the version it makes keeps it until the next
//! transaction begins, [`ooxml_store::DocModel::begin`], as it keeps what it drew), name a
//! block, or stand for block identity: an element is matched here only to replay one
//! transaction's own changes, at the place the later change removes it (the node there is the
//! one the earlier change inserted, equal to what the later one removes but for the values
//! the draws wrote); a block's identity comes from the index (ADR-008).

use super::{attribute_below, set_below, Draw};
use crate::drawn::id_attribute;
use ooxml_xml::{Element, Name, Node, NodePath, Patch, Tree};
use std::sync::Arc;

/// Where a draw wrote: the path below the inserted element and the attribute.
type Slot = (Vec<u32>, Name);

/// One element a splice with draws inserted, as applied, and where its draws wrote.
type Inserted = (Arc<Element>, Vec<Slot>);

/// The elements a transaction's splices with draws inserted. Cloning shares.
#[derive(Debug, Clone, Default)]
pub(crate) struct Redrawn(Arc<Vec<Inserted>>);

impl Redrawn {
    /// Nothing inserted.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The nodes a splice inserted, `applied` with `draws` written.
    pub(crate) fn note(&mut self, applied: &[Node], draws: &[Draw]) {
        let mut noted = Vec::new();
        for (k, node) in applied.iter().enumerate() {
            let Node::Element(a) = node else { continue };
            let slots: Vec<Slot> = draws
                .iter()
                .filter(|d| usize::try_from(d.node).is_ok_and(|n| n == k))
                .map(|d| (d.at.clone(), Arc::clone(&d.attribute)))
                .collect();
            // only paragraph and text ids drawn from a seed: an element with another draw
            // (a revision's number, counted from the version it lands on) is not named
            // again, and a later change replacing it is prepared again
            if slots.is_empty() || slots.iter().any(|(_, name)| !id_attribute(name)) {
                continue;
            }
            noted.push((Arc::clone(a), slots));
        }
        if !noted.is_empty() {
            Arc::make_mut(&mut self.0).extend(noted);
        }
    }

    /// `patch`, a later splice of the transaction applied to `tree` (its part), with each
    /// node it removes that is, at its place in `tree`, an element an earlier splice
    /// inserted, and is that element but for what its draws wrote, named as it is in `tree`; `None` where it names none, or where what it inserts carries, at a place its
    /// own `draws` do not write, a value those draws wrote (made from the element as the
    /// working version had it: replayed, it would write a stale id, so the transaction is
    /// prepared again instead).
    pub(crate) fn patch(&self, patch: &Patch, draws: &[Draw], tree: &Tree) -> Option<Patch> {
        if self.is_empty() {
            return None;
        }
        let mut carried: Vec<String> = Vec::new();
        let mut named = false;
        let mut removed: Vec<Node> = Vec::with_capacity(patch.removed().len());
        for (k, node) in patch.removed().iter().enumerate() {
            let here = element_at(tree, patch.parent(), patch.start() + k);
            let found = match (node, &here) {
                (Node::Element(e), Some(here)) if !Arc::ptr_eq(e, here) && **e != **here => self
                    .0
                    .iter()
                    .find(|(applied, slots)| {
                        Arc::ptr_eq(applied, here)
                            && emptied(e, slots).is_some_and(|mine| Some(mine) == emptied(applied, slots))
                    })
                    .map(|(applied, slots)| (e, applied, slots)),
                _ => None,
            };
            match found {
                Some((e, applied, slots)) => {
                    named = true;
                    carried.extend(
                        slots
                            .iter()
                            .filter_map(|(at, name)| attribute_below(e, at, name))
                            .map(|v| v.trim().to_uppercase()),
                    );
                    removed.push(Node::Element(Arc::clone(applied)));
                }
                None => removed.push(node.clone()),
            }
        }
        if !named || carries(patch.inserted(), &carried, draws) {
            return None;
        }
        Some(Patch::splice(
            patch.parent().clone(),
            patch.start(),
            removed,
            patch.inserted().to_vec(),
        ))
    }
}

/// The element that is child `index` of the element at `parent` in `tree` (`[0]` the root).
fn element_at(tree: &Tree, parent: &NodePath, index: usize) -> Option<Arc<Element>> {
    let (&first, below) = parent.0.split_first()?;
    if first != 0 {
        return None;
    }
    let mut at = tree.root();
    for &i in below {
        at = at.children().get(i)?.as_element()?;
    }
    at.children().get(index)?.as_element().cloned()
}

/// `element` with the attributes at `slots` emptied; `None` where a slot is not there.
fn emptied(element: &Arc<Element>, slots: &[Slot]) -> Option<Arc<Element>> {
    let mut out = Arc::clone(element);
    for (at, name) in slots {
        out = set_below(&out, at, name, "")?;
    }
    Some(out)
}

/// Whether an id attribute in `nodes` that none of `draws` writes holds one of `values`
/// (upper case).
fn carries(nodes: &[Node], values: &[String], draws: &[Draw]) -> bool {
    if values.is_empty() {
        return false;
    }
    for (k, node) in nodes.iter().enumerate() {
        let Node::Element(element) = node else { continue };
        let mut stack: Vec<(&Element, Vec<u32>)> = vec![(element, Vec::new())];
        while let Some((e, path)) = stack.pop() {
            for attribute in e.attributes().iter() {
                let written = || {
                    draws.iter().any(|d| {
                        usize::try_from(d.node).is_ok_and(|n| n == k) && d.at == path && d.attribute == attribute.name
                    })
                };
                if id_attribute(&attribute.name)
                    && values.contains(&attribute.value.trim().to_uppercase())
                    && !written()
                {
                    return true;
                }
            }
            for (j, child) in e.children().iter().enumerate() {
                if let Node::Element(c) = child {
                    let mut below = path.clone();
                    below.push(u32::try_from(j).unwrap_or(u32::MAX));
                    stack.push((c, below));
                }
            }
        }
    }
    false
}
