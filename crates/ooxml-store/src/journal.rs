//! Owns: the [`Journal`], the change set of each of the latest versions published -- commits,
//! undos and redos alike -- and moving a change set made on an older version past them
//! ([`Journal::rebase`]): a commit's past the commits since its snapshot (design section 5),
//! an undo's inverses past the steps after the one it takes back.
//!
//! Must not: decide what is kept for whom; [`crate::Store`] trims it to its rebase window and
//! to what the history can still reach.

use crate::changeset::ChangeSet;
use crate::{DocModel, Seq};
use std::collections::VecDeque;

/// The latest versions published, oldest first: each one's number and the change set that
/// made it from the one before.
pub(crate) struct Journal<D: DocModel> {
    entries: VecDeque<(Seq, ChangeSet<D>)>,
}

impl<D: DocModel> Journal<D> {
    pub(crate) fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    /// Keep the change set that made version `seq`.
    pub(crate) fn push(&mut self, seq: Seq, changes: ChangeSet<D>) {
        self.entries.push_back((seq, changes));
    }

    /// Let go of the oldest entries beyond the latest `window`, but none after `needed` (the
    /// oldest version the history may still rebase from).
    pub(crate) fn trim(&mut self, window: usize, needed: Option<Seq>) {
        while self.entries.len() > window {
            match (self.entries.front(), needed) {
                (Some((seq, _)), Some(needed)) if *seq > needed => break,
                (Some(_), _) => drop(self.entries.pop_front()),
                (None, _) => break,
            }
        }
    }

    /// `changes`, made on version `base`, moved past every change set published since, in
    /// order; `None` when one of them is no longer kept or a change cannot be moved past one
    /// of theirs. Asked only when something was published since `base`.
    pub(crate) fn rebase(&self, base: Seq, mut changes: Vec<D::Change>) -> Option<Vec<D::Change>> {
        let since = self.entries.iter().position(|(seq, _)| *seq > base)?;
        if since == 0 && self.entries.front().is_some_and(|(seq, _)| seq.0 != base.0 + 1) {
            return None; // the versions right after `base` are no longer kept
        }
        for (_, past) in self.entries.iter().skip(since) {
            for past in past.changes()?.iter() {
                for change in &mut changes {
                    *change = D::rebase(change, past).ok()?;
                }
            }
        }
        Some(changes)
    }
}
