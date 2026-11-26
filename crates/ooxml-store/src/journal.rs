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
use crate::{DocModel, First, KeySet, Seq};
use std::collections::VecDeque;
use std::sync::Arc;

/// The latest versions published, oldest first: each one's number, the change set that
/// made it from the one before and the keys that change set wrote, as resolved where it was
/// applied ([`DocModel::resolve`]).
pub(crate) struct Journal<D: DocModel> {
    entries: VecDeque<Arc<Entry<D>>>,
    /// The newest entry let go (`None`: none yet). Numbers need not follow one another (a
    /// reopen, or a held transaction's versions that were read but never published), so
    /// whether the versions right after one are kept is told by it.
    trimmed: Option<Seq>,
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

    /// `changes`, a sequence made on the version these entries were published after, moved
    /// past theirs, in order; `None` when a change cannot be moved past one of theirs, or
    /// one of theirs past a change (or a change set of theirs is no longer kept).
    ///
    /// The invariant: for `changes` C = c1..cn made on version B and their changes P =
    /// p1..pm, the result C' gives B·P·C' = B·C·P'. Each c_i is moved past each p_j as p_j
    /// stands where c_i was made: p_j carried past c1..c(i-1) first. Where two put
    /// something at one place, the one published first -- theirs -- comes first
    /// ([`First`]). With the keys disjoint (checked before), B·P·C' is what preparing C on
    /// B·P gives: the serial result in commit order.
    pub(crate) fn rebase(&self, mut changes: Vec<D::Change>) -> Option<Vec<D::Change>> {
        for entry in &self.0 {
            for past in entry.1.changes()?.iter() {
                let mut past = past.clone();
                let last = changes.len().saturating_sub(1);
                for (i, change) in changes.iter_mut().enumerate() {
                    let moved = D::rebase(change, &past, First::Past).ok()?;
                    if i < last {
                        // where the next change was made: after this one
                        past = D::rebase(&past, change, First::Change).ok()?;
                    }
                    *change = moved;
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
            trimmed: None,
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
                (Some(_), _) => {
                    self.trimmed = self.entries.pop_front().map(|entry| entry.0);
                }
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
        if since == 0 && self.trimmed.is_some_and(|trimmed| trimmed > base) {
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
