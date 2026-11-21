//! Owns: the history of a store (design section 4, Undo): the steps undo can reach, the
//! steps undone, the entanglement rule, and how an undo or redo computes its new content and
//! the change set that makes it: a scoped undo's inverses, and a redo's, moved past what was
//! published since ([`Journal::rebase`]), so a step can be taken back while later steps
//! moved its content.
//!
//! Must not: publish or lock; [`crate::Store`] runs these plans in its critical section.
//! Undo and redo make new versions with new numbers; they never restore an old number.

use crate::changeset::{inverses, ChangeSet};
use crate::journal::Journal;
use crate::{DocModel, Error, KeySet, Seq, Snapshot, StoreError};
use std::collections::VecDeque;
use std::sync::Arc;

/// One committed step: the versions around it and what it read, wrote and changed.
#[derive(Clone)]
pub(crate) struct Step<D: DocModel> {
    pub(crate) before: Snapshot<D>,
    pub(crate) after: Snapshot<D>,
    pub(crate) changes: ChangeSet<D>,
    pub(crate) reads: KeySet<D::Key>,
    pub(crate) writes: KeySet<D::Key>,
    pub(crate) caller: Option<Arc<str>>,
}

/// A step taken out, with the undo that took it out: the version it was applied to, its
/// change set, and the version it made.
struct Undone<D: DocModel> {
    step: Step<D>,
    on: Snapshot<D>,
    changes: ChangeSet<D>,
    result: Snapshot<D>,
    /// The undo's content was the step's "before" itself (the head was still its "after").
    exact: bool,
}

/// A step as [`crate::Store::steps`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepInfo<K: Ord> {
    /// The version the step made; the step's name in [`crate::Store::undo_step`].
    pub step: Seq,
    /// The version it was made on.
    pub from: Seq,
    /// Who made it.
    pub caller: Option<Arc<str>>,
    /// What it read.
    pub reads: KeySet<K>,
    /// What it wrote.
    pub writes: KeySet<K>,
}

/// What an undo or redo did.
#[derive(Debug)]
pub struct HistoryOutcome<D: DocModel> {
    /// The version it made.
    pub version: Snapshot<D>,
    /// The head it was made from.
    pub previous: Snapshot<D>,
    /// The step undone or redone, by the version it made when committed.
    pub step: Seq,
    /// The keys it wrote.
    pub writes: KeySet<D::Key>,
}

/// Which step a plan acts on.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Target {
    /// `undo[index]`; `linear` for plain undo (its redo is dropped by the next commit);
    /// `exact` when the new content is the step's "before" itself.
    Undo { index: usize, linear: bool, exact: bool },
    /// `redo[index]` when `linear`, else `undone[index]`.
    Redo { index: usize, linear: bool },
}

/// An undo or redo worked out against the head, ready to publish.
pub(crate) struct Planned<D: DocModel> {
    pub(crate) target: Target,
    pub(crate) step: Seq,
    /// The state number of the content it brings back, when that is a state that was
    /// ([`crate::Version::number`]); `None`: a new state, a new number.
    pub(crate) number: Option<u64>,
    pub(crate) doc: D,
    /// The change set that makes `doc` from the head.
    pub(crate) changes: ChangeSet<D>,
    pub(crate) writes: KeySet<D::Key>,
}

/// What a critical section took out of the history; dropped after the lock is released,
/// because freeing old versions can take a while.
pub(crate) struct Garbage<D: DocModel> {
    _steps: Vec<Step<D>>,
    _versions: Vec<Snapshot<D>>,
}

impl<D: DocModel> Garbage<D> {
    /// Plain redo's steps, dropped.
    fn take_redo(&mut self, redo: &mut Vec<Undone<D>>) {
        for undone in redo.drain(..) {
            self._versions.extend([undone.result, undone.on]);
            self._steps.push(undone.step);
        }
    }
}

pub(crate) struct History<D: DocModel> {
    undo: VecDeque<Step<D>>,
    /// Plain undo's steps, newest last; a new commit drops them.
    redo: Vec<Undone<D>>,
    /// Scoped undo's steps, oldest first; kept across commits, up to the depth.
    undone: VecDeque<Undone<D>>,
    depth: usize,
}

impl<D: DocModel> History<D> {
    pub(crate) fn new(depth: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            undone: VecDeque::new(),
            depth,
        }
    }

    pub(crate) fn steps(&self) -> Vec<StepInfo<D::Key>> {
        self.undo.iter().map(info).collect()
    }

    pub(crate) fn redoable(&self) -> Vec<StepInfo<D::Key>> {
        self.redo.iter().map(|undone| info(&undone.step)).collect()
    }

    /// The kept version numbered `number` nearest the head: the undo list's newest first
    /// (each step's "after", then its "before"), then plain redo's next first.
    pub(crate) fn numbered(&self, number: u64) -> Option<Snapshot<D>> {
        let undo = self.undo.iter().rev().flat_map(|step| [&step.after, &step.before]);
        let redo = self
            .redo
            .iter()
            .rev()
            .flat_map(|undone| [&undone.step.after, &undone.result]);
        undo.chain(redo).find(|version| version.number() == number).cloned()
    }

    /// The newest step `caller` made that undo can reach.
    pub(crate) fn latest_by(&self, caller: &str) -> Option<Seq> {
        self.undo
            .iter()
            .rev()
            .find(|step| step.caller.as_deref() == Some(caller))
            .map(|step| step.after.seq())
    }

    /// A new commit: the step goes on the undo list and plain redo is dropped.
    pub(crate) fn commit(&mut self, step: Step<D>) -> Garbage<D> {
        let mut garbage = self.push(step);
        garbage.take_redo(&mut self.redo);
        garbage
    }

    fn push(&mut self, step: Step<D>) -> Garbage<D> {
        self.undo.push_back(step);
        let excess = self.undo.len().saturating_sub(self.depth);
        Garbage {
            _steps: self.undo.drain(..excess).collect(),
            _versions: Vec::new(),
        }
    }

    /// The oldest version a plan may still move a change set from: the journal keeps what
    /// was published after it.
    pub(crate) fn oldest_reach(&self) -> Option<Seq> {
        let steps = self.undo.iter().map(|step| step.after.seq());
        let undone = self.redo.iter().chain(self.undone.iter()).map(|u| u.result.seq());
        steps.chain(undone).min()
    }

    /// Plan undoing step `at` (the newest when `None`), refusing it when entangled.
    pub(crate) fn plan_undo(
        &self,
        at: Option<Seq>,
        shared: &KeySet<D::Key>,
        head: &Snapshot<D>,
        journal: &Journal<D>,
    ) -> Result<Planned<D>, Error<D>> {
        let index = match at {
            None => self.undo.len().checked_sub(1).ok_or(StoreError::NothingToUndo)?,
            Some(seq) => self
                .undo
                .iter()
                .position(|step| step.after.seq() == seq)
                .ok_or(StoreError::UnknownStep { step: seq })?,
        };
        let step = self.undo.get(index).ok_or(StoreError::NothingToUndo)?;
        // Every change since the step is a later step still in this list, or the undo of one
        // (which put its keys back): only the later steps here can depend on it.
        let mut overlap = step.writes.intersection(shared);
        for later in self.undo.iter().skip(index + 1) {
            overlap.extend(&step.writes.intersection(&later.reads));
            overlap.extend(&step.writes.intersection(&later.writes));
        }
        let overlap = match overlap {
            KeySet::Keys(keys) => KeySet::Keys(keys.into_iter().filter(|key| !D::kept(key)).collect()),
            KeySet::Everything => KeySet::Everything,
        };
        if !overlap.is_empty() {
            return Err(StoreError::Entangled {
                step: step.after.seq(),
                overlap,
            });
        }
        let exact = same(head, &step.after);
        let (doc, changes, writes) = revert(step, head, journal)?;
        Ok(Planned {
            target: Target::Undo {
                index,
                linear: at.is_none(),
                exact,
            },
            step: step.after.seq(),
            number: (at.is_none() && exact).then(|| step.before.number()),
            doc,
            changes,
            writes,
        })
    }

    /// Plan redoing step `at` (plain redo's newest when `None`), refusing it when what it
    /// read or wrote holds other content than its undo left.
    pub(crate) fn plan_redo(
        &self,
        at: Option<Seq>,
        head: &Snapshot<D>,
        journal: &Journal<D>,
    ) -> Result<Planned<D>, Error<D>> {
        let (index, linear) = match at {
            None => (self.redo.len().checked_sub(1).ok_or(StoreError::NothingToRedo)?, true),
            Some(seq) => {
                let made = |undone: &Undone<D>| undone.step.after.seq() == seq;
                match self.undone.iter().position(made) {
                    Some(index) => (index, false),
                    None => (
                        self.redo
                            .iter()
                            .position(made)
                            .ok_or(StoreError::UnknownStep { step: seq })?,
                        true,
                    ),
                }
            }
        };
        let undone = if linear {
            self.redo.get(index)
        } else {
            self.undone.get(index)
        };
        let undone = undone.ok_or(StoreError::NothingToRedo)?;
        let step = &undone.step;
        // Its changes go on top of the head: what it read and wrote must be as its undo
        // left them (written since and different; a key written and put back is fine).
        let mut depends = step.reads.clone();
        depends.extend(&step.writes);
        let overlap = match head.keys_written_since(undone.result.seq(), &depends) {
            KeySet::Everything if !head.doc().agrees(undone.result.doc(), &KeySet::Everything) => KeySet::Everything,
            KeySet::Everything => KeySet::new(),
            KeySet::Keys(keys) => KeySet::of(
                keys.into_iter()
                    .filter(|key| !head.doc().agrees(undone.result.doc(), &KeySet::of([key.clone()]))),
            ),
        };
        if !overlap.is_empty() {
            return Err(StoreError::Entangled {
                step: step.after.seq(),
                overlap,
            });
        }
        let (doc, changes) = reapply(undone, head, journal)?;
        let number = (linear && same(head, &undone.result)).then(|| step.after.number());
        Ok(Planned {
            target: Target::Redo { index, linear },
            step: step.after.seq(),
            number,
            doc,
            changes,
            writes: step.writes.clone(),
        })
    }

    /// Record a published plan: `head` is the version it was made on, `version` the one
    /// it made, `changes` its change set.
    pub(crate) fn record(
        &mut self,
        target: Target,
        head: Snapshot<D>,
        version: Snapshot<D>,
        changes: ChangeSet<D>,
    ) -> Garbage<D> {
        let mut garbage = Garbage {
            _steps: Vec::new(),
            _versions: Vec::new(),
        };
        match target {
            Target::Undo { index, linear, exact } => {
                if let Some(step) = self.undo.remove(index) {
                    let undone = Undone {
                        step,
                        on: head,
                        changes,
                        result: version,
                        exact,
                    };
                    if linear {
                        self.redo.push(undone);
                    } else {
                        self.undone.push_back(undone);
                        let excess = self.undone.len().saturating_sub(self.depth);
                        for gone in self.undone.drain(..excess) {
                            garbage._versions.extend([gone.result, gone.on]);
                        }
                    }
                }
            }
            Target::Redo { index, linear } => {
                let undone = if linear {
                    (index < self.redo.len()).then(|| self.redo.remove(index))
                } else {
                    self.undone.remove(index)
                };
                if let Some(Undone { step, result, on, .. }) = undone {
                    garbage._versions.extend([result, on]);
                    let redone = Step {
                        before: head,
                        after: version,
                        changes,
                        ..step
                    };
                    garbage._steps.extend(self.push(redone)._steps);
                }
            }
        }
        garbage
    }
}

fn info<D: DocModel>(step: &Step<D>) -> StepInfo<D::Key> {
    StepInfo {
        step: step.after.seq(),
        from: step.before.seq(),
        caller: step.caller.clone(),
        reads: step.reads.clone(),
        writes: step.writes.clone(),
    }
}

/// Whether `head` holds `version`'s content: the very version, or one that agrees with it
/// everywhere (an undo's exact result, say, of the same content as an earlier version).
/// What no step takes back ([`DocModel::carry`]) is not compared.
fn same<D: DocModel>(head: &Snapshot<D>, version: &Snapshot<D>) -> bool {
    Arc::ptr_eq(head, version) || head.doc().agrees(version.doc(), &KeySet::Everything)
}

/// What a plan makes: the new content, its change set from the head, the keys it writes.
type Made<D> = (D, ChangeSet<D>, KeySet<<D as DocModel>::Key>);

/// `changes`, made on version `base`, moved past what was published since and applied to
/// `head`; a change that cannot be moved past a later one is entangled with it.
fn moved<D: DocModel>(
    step: Seq,
    base: Seq,
    changes: Vec<D::Change>,
    head: &Snapshot<D>,
    journal: &Journal<D>,
    writes: &KeySet<D::Key>,
) -> Result<(D, Vec<D::Change>), Error<D>> {
    let entangled = || StoreError::Entangled {
        step,
        overlap: writes.clone(),
    };
    let changes = journal.rebase(base, changes).ok_or_else(entangled)?;
    let mut doc = head.doc().clone();
    doc.begin();
    for change in &changes {
        doc.apply(change).map_err(|_| entangled())?;
    }
    Ok((doc, changes))
}

/// The head without `step`: the step's own "before" when the head is still its "after"
/// (its change set the step's inverses, worked out when asked), otherwise its changes'
/// inverses moved past the versions since and applied to the head.
fn revert<D: DocModel>(step: &Step<D>, head: &Snapshot<D>, journal: &Journal<D>) -> Result<Made<D>, Error<D>> {
    let mut writes = step.writes.clone();
    if same(head, &step.after) {
        let changes = ChangeSet::inverse_of(step.before.doc().clone(), step.changes.clone());
        return Ok((step.before.doc().clone(), changes, writes));
    }
    let made = step.changes.changes().ok_or(StoreError::Entangled {
        step: step.after.seq(),
        overlap: KeySet::Everything,
    })?;
    let inverses = inverses(step.before.doc(), &made)?;
    for inverse in &inverses {
        D::written(inverse, &mut writes);
    }
    let (doc, changes) = moved(step.after.seq(), step.after.seq(), inverses, head, journal, &writes)?;
    Ok((doc, ChangeSet::made(changes.into()), writes))
}

/// The head with `undone`'s step again: the step's own "after" and changes when the head is
/// still what an exact undo made, otherwise the undo's own inverses moved past the versions
/// since and applied to the head.
fn reapply<D: DocModel>(
    undone: &Undone<D>,
    head: &Snapshot<D>,
    journal: &Journal<D>,
) -> Result<(D, ChangeSet<D>), Error<D>> {
    if undone.exact && same(head, &undone.result) {
        return Ok((undone.step.after.doc().clone(), undone.step.changes.clone()));
    }
    let step = undone.step.after.seq();
    let made = undone.changes.changes().ok_or(StoreError::Entangled {
        step,
        overlap: KeySet::Everything,
    })?;
    let inverses = inverses(undone.on.doc(), &made)?;
    let (doc, changes) = if Arc::ptr_eq(head, &undone.result) {
        let mut doc = head.doc().clone();
        doc.begin();
        for change in &inverses {
            doc.apply(change).map_err(StoreError::Model)?;
        }
        (doc, inverses)
    } else {
        moved(step, undone.result.seq(), inverses, head, journal, &undone.step.writes)?
    };
    Ok((doc, ChangeSet::made(changes.into())))
}
