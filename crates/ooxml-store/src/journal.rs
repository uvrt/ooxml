//! Owns: the [`Journal`], the change set of each of the latest versions published -- commits,
//! undos and redos alike -- and moving a change set made on an older version past them
//! ([`Journal::rebase`]): a commit's past the commits since its snapshot (design section 5),
//! an undo's inverses past the steps after the one it takes back; and the keys those change
//! sets wrote (what a read by condition is checked against).
//!
//! A commit takes the entries since its snapshot under the commit lock ([`Journal::since_base`],
//! shared, not copied) and checks and rebases past them outside it ([`Since`]).
//!
//! Must not: decide what is kept for whom; [`crate::Store`] trims it to its rebase window and
//! to what the history can still reach.

use crate::changeset::ChangeSet;
use crate::{DocModel, KeySet, Seq};
use std::collections::VecDeque;
use std::sync::Arc;

/// The latest versions published, oldest first: each one's number, the change set that
/// made it from the one before and the keys that change set wrote, as resolved where it was
/// applied ([`DocModel::resolve`]).
pub(crate) struct Journal<D: DocModel> {
    entries: VecDeque<Arc<Entry<D>>>,
}

/// One version's entry: its number, the change set that made it, the keys that wrote.
type Entry<D> = (Seq, ChangeSet<D>, KeySet<<D as DocModel>::Key>);

/// The entries published after a version, in order, held apart from the journal: what a
/// commit checks and rebases against outside the commit lock.
pub(crate) struct Since<D: DocModel>(Vec<Arc<Entry<D>>>);

impl<D: DocModel> Since<D> {
    /// None published since.
    pub(crate) fn none() -> Self {
        Self(Vec::new())
    }

    /// The keys they wrote, as resolved where each was applied.
    pub(crate) fn written(&self) -> KeySet<D::Key> {
        let mut written = KeySet::new();
        for entry in &self.0 {
            written.extend(&entry.2);
        }
        written
    }

    /// `changes` moved past theirs, in order; `None` when a change cannot be moved past one
    /// of theirs (or a change set of theirs is no longer kept).
    pub(crate) fn rebase(&self, mut changes: Vec<D::Change>) -> Option<Vec<D::Change>> {
        for entry in &self.0 {
            for past in entry.1.changes()?.iter() {
                for change in &mut changes {
                    *change = D::rebase(change, past).ok()?;
                }
            }
        }
        Some(changes)
    }
}

impl<D: DocModel> Journal<D> {
    pub(crate) fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    /// Keep the change set that made version `seq`, and the keys it wrote.
    pub(crate) fn push(&mut self, seq: Seq, changes: ChangeSet<D>, writes: KeySet<D::Key>) {
        self.entries.push_back(Arc::new((seq, changes, writes)));
    }

    /// Let go of the oldest entries beyond the latest `window`, but none after `needed` (the
    /// oldest version the history may still rebase from).
    pub(crate) fn trim(&mut self, window: usize, needed: Option<Seq>) {
        while self.entries.len() > window {
            match (self.entries.front(), needed) {
                (Some(entry), Some(needed)) if entry.0 > needed => break,
                (Some(_), _) => drop(self.entries.pop_front()),
                (None, _) => break,
            }
        }
    }

    /// The entries published since version `base`, shared; `None` when the versions right
    /// after `base` are no longer kept.
    pub(crate) fn since_base(&self, base: Seq) -> Option<Since<D>> {
        Some(Since(self.since_entries(base)?.cloned().collect()))
    }

    /// The entries published since version `base`, in order; `None` when the versions
    /// right after `base` are no longer kept. Asked only when something was published since.
    fn since_entries(&self, base: Seq) -> Option<impl Iterator<Item = &Arc<Entry<D>>>> {
        let since = self.entries.iter().position(|entry| entry.0 > base)?;
        if since == 0 && self.entries.front().is_some_and(|entry| entry.0 .0 != base.0 + 1) {
            return None; // the versions right after `base` are no longer kept
        }
        Some(self.entries.iter().skip(since))
    }

    /// `changes`, made on version `base`, moved past every change set published since, in
    /// order; `None` when one of them is no longer kept or a change cannot be moved past one
    /// of theirs. Asked only when something was published since `base`.
    pub(crate) fn rebase(&self, base: Seq, changes: Vec<D::Change>) -> Option<Vec<D::Change>> {
        self.since_base(base)?.rebase(changes)
    }
}
