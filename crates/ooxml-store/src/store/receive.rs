//! Owns: [`Store::receive`], another editor's commit coming in (co-authoring refresh): made on
//! a version this store published (the one the other editor's save was made from), it is the
//! server's accepted version, so it always lands. The steps this store made since that
//! version which touch what it changed are taken back first (scoped undos, each a step of its
//! own that can be redone by hand) and named ([`HandedBack`]); the others stay, and the
//! commit is prepared on the head they leave and published as one step by a foreign caller
//! ([`Caller::foreign`]). It runs behind the gate, so no commit of this store's lands between
//! the take-backs and the commit; a transaction prepared before it that read what it changed
//! is refused at its commit ([`crate::StoreError::ForeignChange`]).
//!
//! Must not: decide what the other editor changed (the format does, in `touches` and
//! `prepare`), or refuse a commit that conflicts with this store's steps: those are handed
//! back instead (main's ruling for CF2: the server's version is the truth).

use super::{Conflict, Store};
use crate::txn::Ids;
use crate::{Caller, Committed, DocModel, Error, KeySet, Seq, Snapshot, StepInfo, StoreError, Txn};

/// A step of this store's taken back because another editor's commit changed what it
/// touched: the step as it was, and the version its undo made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandedBack<K: Ord> {
    /// The step taken back.
    pub step: StepInfo<K>,
    /// The version the undo that took it back made (redo it by this, [`Store::redo_step`]
    /// names it by `step.step`).
    pub undo: Seq,
}

/// What a preparation of another editor's commit made ([`Store::receive`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prepared {
    /// The change, ready to commit.
    Made,
    /// Nothing yet: what the change touches grew (a part it now takes whole), so the steps
    /// that touch it are taken back again before it is prepared again.
    Widened,
}

/// What another editor's commit did ([`Store::receive`]).
#[derive(Debug)]
pub struct Received<D: DocModel> {
    /// The commit: its version, the head it was published after, the keys it wrote.
    pub committed: Committed<D, ()>,
    /// This store's steps taken back first, newest first, as they were taken back.
    pub handed_back: Vec<HandedBack<D::Key>>,
}

impl<D: DocModel> Store<D> {
    /// The version `seq`: the head, or one the history keeps with every step after it (its
    /// undo list's oldest step made on it or later); `None` otherwise.
    pub fn kept(&self, seq: Seq) -> Option<Snapshot<D>> {
        let head = self.snapshot();
        if head.seq() == seq {
            return Some(head);
        }
        self.lock_state().history.kept(seq)
    }

    /// Commit another editor's change, made on version `base`, as `editor` (a foreign
    /// caller). `touches` says whether one of this store's steps since `base` touched what the
    /// change changes; `prepare` makes the change on the transaction's head, given `base`, or
    /// says that what it touches grew ([`Prepared::Widened`]).
    ///
    /// Each step since `base` that `touches`, and each later step entangled with one of
    /// those (it read or wrote what one wrote), is taken back by a scoped undo, newest first;
    /// then `prepare` runs on the head behind the gate and its change is published, prepared
    /// again only if an undo slipped in meanwhile.
    ///
    /// # Errors
    /// [`StoreError::NotKept`] when `base` is not kept with the steps after it; a take-back
    /// the history refuses (another editor's step in the way); what `prepare` returned.
    pub fn receive<E: From<Error<D>>>(
        &self,
        editor: &Caller,
        base: Seq,
        setup: &D::Setup,
        touches: impl Fn(&StepInfo<D::Key>) -> bool,
        prepare: impl Fn(&mut Txn<'_, D>, &Snapshot<D>) -> Result<Prepared, E>,
    ) -> Result<Received<D>, E> {
        let start = self.clock.monotonic();
        let _gate = self.enter_gate();
        let on = self.kept(base).ok_or(StoreError::NotKept { seq: base })?;
        let mut handed_back = self.take_back_since(base, &touches)?;
        let mut attempts = 0;
        loop {
            attempts += 1;
            let mut tx = Txn::new(self.snapshot(), Ids::Borrowed(&self.ids), setup).behind_gate();
            if prepare(&mut tx, &on)? == Prepared::Widened {
                handed_back.extend(self.take_back_since(base, &touches)?);
                continue;
            }
            match self.try_commit(editor, tx.into_parts(), true, None) {
                Ok(done) => {
                    let committed = self.committed(done, (), attempts, true, start);
                    return Ok(Received { committed, handed_back });
                }
                // an undo or redo slipped in through the gate: prepared again on the head
                Err(Conflict { .. }) => self.counts.conflict(),
            }
        }
    }

    /// Take back, newest first, the steps since `base` that `touches`, with every later
    /// step entangled with one of them.
    fn take_back_since(
        &self,
        base: Seq,
        touches: &impl Fn(&StepInfo<D::Key>) -> bool,
    ) -> Result<Vec<HandedBack<D::Key>>, Error<D>> {
        let steps: Vec<StepInfo<D::Key>> = self.steps().into_iter().filter(|s| s.step > base).collect();
        let mut taken: Vec<&StepInfo<D::Key>> = Vec::new();
        for step in steps.iter().filter(|s| !s.foreign) {
            // as the history finds a step entangled with a later one (`History::plan_undo`)
            let entangled = taken.iter().any(|t| {
                let mut with = t.writes.intersection(&step.reads);
                with.extend(&t.writes.intersection(&step.writes).without_where(D::derived));
                !with.without_where(D::kept).is_empty()
            });
            if entangled || touches(step) {
                taken.push(step);
            }
        }
        let mut handed_back = Vec::with_capacity(taken.len());
        for step in taken.into_iter().rev() {
            let outcome = self.undo_step(step.step, &KeySet::new())?;
            handed_back.push(HandedBack {
                step: step.clone(),
                undo: outcome.version.seq(),
            });
        }
        Ok(handed_back)
    }
}
