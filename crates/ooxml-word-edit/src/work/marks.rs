//! Owns: the characters of a node's text an edit wrote ("marks", docx-agent's typography
//! `Recorder` within one tree): set as text is spelled, carried along when a node's text is
//! split or joined, read back by place.
//!
//! Must not: know what a mark is used for, or carry marks to a copy of a node (docx-agent's
//! recorder holds nodes, and a copy is a node of its own).

use super::{Id, Work};
use std::collections::BTreeSet;

impl Work {
    /// Mark the characters `offsets` of `node`'s text as written, in place of any marks it
    /// had (those past its text are left out).
    pub(crate) fn mark(&mut self, node: Id, offsets: impl IntoIterator<Item = usize>) {
        let length = self.text(node).chars().count();
        let found: BTreeSet<usize> = offsets.into_iter().filter(|k| *k < length).collect();
        if found.is_empty() {
            self.marks.remove(&node);
        } else {
            self.marks.insert(node, found);
        }
    }

    /// Whether character `offset` of `node`'s text is marked written.
    pub(crate) fn marked(&self, node: Id, offset: usize) -> bool {
        self.marks.get(&node).is_some_and(|m| m.contains(&offset))
    }

    /// The nodes with marks.
    pub(crate) fn marked_nodes(&self) -> impl Iterator<Item = Id> + '_ {
        self.marks.keys().copied()
    }

    /// Whether any node is marked.
    pub(crate) fn has_marks(&self) -> bool {
        !self.marks.is_empty()
    }

    /// `node`'s text `[..at]` moved to `left` (before it): its marks go with it.
    pub(crate) fn split_marks(&mut self, node: Id, left: Id, at: usize) {
        let Some(held) = self.marks.remove(&node) else {
            return;
        };
        let (before, after): (BTreeSet<usize>, BTreeSet<usize>) = held.into_iter().partition(|k| *k < at);
        self.mark(left, before);
        self.mark(node, after.into_iter().map(|k| k - at));
    }

    /// `gone`'s text appended to `kept`'s, which was `shift` characters long.
    pub(crate) fn join_marks(&mut self, kept: Id, gone: Id, shift: usize) {
        let moved = self.marks.remove(&gone).unwrap_or_default();
        if moved.is_empty() {
            return;
        }
        let mut all = self.marks.remove(&kept).unwrap_or_default();
        all.extend(moved.into_iter().map(|k| k + shift));
        self.mark(kept, all);
    }
}
