//! Owns: the history of a store (design section 4, Undo): the steps undo can reach, the
//! steps undone, the entanglement rule, and how an undo or redo computes its new content.
//!
//! Must not: publish or lock; [`crate::Store`] runs these plans in its critical section.
//! Undo and redo make new versions with new numbers; they never restore an old number.

use crate::{DocModel, Error, KeySet, Seq, Snapshot, StoreError};
use std::collections::VecDeque;
use std::sync::Arc;

/// One committed step: the versions around it and what it read, wrote and changed.
#[derive(Clone)]
pub(crate) struct Step<D: DocModel> {
    pub(crate) before: Snapshot<D>,
    pub(crate) after: Snapshot<D>,
    pub(crate) changes: Arc<[D::Change]>,
    pub(crate) reads: KeySet<D::Key>,
    pub(crate) writes: KeySet<D::Key>,
    pub(crate) caller: Option<Arc<str>>,
}

/// A step taken out, with the version its undo made.
struct Undone<D: DocModel> {
    step: Step<D>,
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
    pub(crate) doc: D,
    pub(crate) writes: KeySet<D::Key>,
}

/// What a critical section took out of the history; dropped after the lock is released,
/// because freeing old versions can take a while.
pub(crate) struct Garbage<D: DocModel> {
    _steps: Vec<Step<D>>,
    _versions: Vec<Snapshot<D>>,
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
        for undone in self.redo.drain(..) {
            garbage._versions.push(undone.result);
            garbage._steps.push(undone.step);
        }
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

    /// Plan undoing step `at` (the newest when `None`), refusing it when entangled.
    pub(crate) fn plan_undo(
        &self,
        at: Option<Seq>,
        shared: &KeySet<D::Key>,
        head: &Snapshot<D>,
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
        if !overlap.is_empty() {
            return Err(StoreError::Entangled {
                step: step.after.seq(),
                overlap,
            });
        }
        let exact = Arc::ptr_eq(head, &step.after);
        let (doc, writes) = revert(step, head)?;
        Ok(Planned {
            target: Target::Undo {
                index,
                linear: at.is_none(),
                exact,
            },
            step: step.after.seq(),
            doc,
            writes,
        })
    }

    /// Plan redoing step `at` (plain redo's newest when `None`), refusing it when what it
    /// read or wrote holds other content than its undo left.
    pub(crate) fn plan_redo(&self, at: Option<Seq>, head: &Snapshot<D>) -> Result<Planned<D>, Error<D>> {
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
        let doc = reapply(undone, head)?;
        Ok(Planned {
            target: Target::Redo { index, linear },
            step: step.after.seq(),
            doc,
            writes: step.writes.clone(),
        })
    }

    /// Record a published plan: `head` is the version it was made on, `version` the one
    /// it made.
    pub(crate) fn record(&mut self, target: Target, head: Snapshot<D>, version: Snapshot<D>) -> Garbage<D> {
        let mut garbage = Garbage {
            _steps: Vec::new(),
            _versions: Vec::new(),
        };
        match target {
            Target::Undo { index, linear, exact } => {
                if let Some(step) = self.undo.remove(index) {
                    let undone = Undone {
                        step,
                        result: version,
                        exact,
                    };
                    if linear {
                        self.redo.push(undone);
                    } else {
                        self.undone.push_back(undone);
                        let excess = self.undone.len().saturating_sub(self.depth);
                        garbage._versions.extend(self.undone.drain(..excess).map(|u| u.result));
                    }
                }
            }
            Target::Redo { index, linear } => {
                let undone = if linear {
                    (index < self.redo.len()).then(|| self.redo.remove(index))
                } else {
                    self.undone.remove(index)
                };
                if let Some(Undone { step, result, .. }) = undone {
                    garbage._versions.push(result);
                    let redone = Step {
                        before: head,
                        after: version,
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

/// The head without `step`: the step's own "before" when the head is still its "after",
/// otherwise its changes' inverses applied to the head, newest first.
fn revert<D: DocModel>(step: &Step<D>, head: &Snapshot<D>) -> Result<(D, KeySet<D::Key>), Error<D>> {
    if Arc::ptr_eq(head, &step.after) {
        return Ok((step.before.doc().clone(), step.writes.clone()));
    }
    let mut at = step.before.doc().clone();
    let mut inverses = Vec::with_capacity(step.changes.len());
    for change in step.changes.iter() {
        inverses.push(at.inverse(change).map_err(StoreError::Model)?);
        at.apply(change).map_err(StoreError::Model)?;
    }
    let mut doc = head.doc().clone();
    let mut writes = step.writes.clone();
    for inverse in inverses.iter().rev() {
        doc.apply(inverse).map_err(StoreError::Model)?;
        D::written(inverse, &mut writes);
    }
    Ok((doc, writes))
}

/// The head with `undone`'s step again: the step's own "after" when the head is still
/// what an exact undo made, otherwise its changes applied to the head.
fn reapply<D: DocModel>(undone: &Undone<D>, head: &Snapshot<D>) -> Result<D, Error<D>> {
    if undone.exact && Arc::ptr_eq(head, &undone.result) {
        return Ok(undone.step.after.doc().clone());
    }
    let mut doc = head.doc().clone();
    for change in undone.step.changes.iter() {
        doc.apply(change).map_err(StoreError::Model)?;
    }
    Ok(doc)
}
