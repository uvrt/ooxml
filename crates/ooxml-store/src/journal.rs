//! Owns: the [`Journal`], the change set of each of the latest versions published -- commits,
//! undos and redos alike -- and moving a change set made on an older version past them
//! ([`Journal::rebase`]): a commit's past the commits since its snapshot (design section 5),
//! an undo's inverses past the steps after the one it takes back; and the keys those change
//! sets wrote (what a read by condition is checked against).
//!
//! Must not: decide what is kept for whom; [`crate::Store`] trims it to its rebase window and
//! to what the history can still reach.

use crate::changeset::ChangeSet;
use crate::{DocModel, KeySet, Seq};
use std::collections::VecDeque;

/// The latest versions published, oldest first: each one's number, the change set that
/// made it from the one before and the keys that change set wrote, as resolved where it was
/// applied ([`DocModel::resolve`]).
pub(crate) struct Journal<D: DocModel> {
    entries: VecDeque<Entry<D>>,
}

/// One version's entry: its number, the change set that made it, the keys that wrote.
type Entry<D> = (Seq, ChangeSet<D>, KeySet<<D as DocModel>::Key>);

impl<D: DocModel> Journal<D> {
    pub(crate) fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    /// Keep the change set that made version `seq`, and the keys it wrote.
    pub(crate) fn push(&mut self, seq: Seq, changes: ChangeSet<D>, writes: KeySet<D::Key>) {
        self.entries.push_back((seq, changes, writes));
    }

    /// Let go of the oldest entries beyond the latest `window`, but none after `needed` (the
    /// oldest version the history may still rebase from).
    pub(crate) fn trim(&mut self, window: usize, needed: Option<Seq>) {
        while self.entries.len() > window {
            match (self.entries.front(), needed) {
                (Some((seq, _, _)), Some(needed)) if *seq > needed => break,
                (Some(_), _) => drop(self.entries.pop_front()),
                (None, _) => break,
            }
        }
    }

    /// The keys written by every change set published since version `base`, as resolved
    /// where each was applied; `None` when one of them is no longer kept.
    pub(crate) fn written_since(&self, base: Seq) -> Option<KeySet<D::Key>> {
        let mut written = KeySet::new();
        for (_, _, writes) in self.since_entries(base)? {
            written.extend(writes);
        }
        Some(written)
    }

    /// The change sets published since version `base`, in order; `None` when the versions
    /// right after `base` are no longer kept. Asked only when something was published since.
    fn since(&self, base: Seq) -> Option<impl Iterator<Item = &ChangeSet<D>>> {
        Some(self.since_entries(base)?.map(|(_, changes, _)| changes))
    }

    /// The entries published since version `base`, in order (as [`Journal::since`]).
    fn since_entries(&self, base: Seq) -> Option<impl Iterator<Item = &Entry<D>>> {
        let since = self.entries.iter().position(|(seq, _, _)| *seq > base)?;
        if since == 0 && self.entries.front().is_some_and(|(seq, _, _)| seq.0 != base.0 + 1) {
            return None; // the versions right after `base` are no longer kept
        }
        Some(self.entries.iter().skip(since))
    }

    /// `changes`, made on version `base`, moved past every change set published since, in
    /// order; `None` when one of them is no longer kept or a change cannot be moved past one
    /// of theirs. Asked only when something was published since `base`.
    pub(crate) fn rebase(&self, base: Seq, mut changes: Vec<D::Change>) -> Option<Vec<D::Change>> {
        for past in self.since(base)? {
            for past in past.changes()?.iter() {
                for change in &mut changes {
                    *change = D::rebase(change, past).ok()?;
                }
            }
        }
        Some(changes)
    }
}
