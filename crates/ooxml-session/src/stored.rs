//! Owns: a document's versions as every format keeps them (design phase 3, section 3):
//! [`Stored`], over the store of a format's model -- the head (for the thread whose step
//! group holds the document, the version its ops left), numbered states, undo and redo and a
//! caller's own undo and redo ([`crate::history`]'s rule), who made each step
//! ([`crate::owners`]), the state numbers a host goes on from, step groups -- and
//! [`StoredDoc`], the same erased, as the session reads any document's. A format supplies
//! only its model and what holds the model's store ([`Holds`]: the store itself, or Word's
//! editor), and is told as its versions are made and numbered ([`Holds::noted`],
//! [`Holds::numbered`]).
//!
//! Must not: know a format's model or state beyond [`Holds`], word a reply, or plan an undo
//! itself (the store does).

use crate::batch_run::{BatchCommitted, BatchOps, BatchRefused, OpRan};
use crate::format::AnyVersion;
use crate::history::{numbers_of, Direction, Numbers};
use crate::model::PackageDoc;
use crate::owners::{OwnUndo, Owners, StepOwner};
use crate::SessionError;
use ooxml_opc::Package;
use ooxml_store::{
    AttemptState, Caller, DocModel, Error, HistoryOutcome, InOp, LockTimes, OpTxn, Seq, Snapshot, Store, StoreError,
    StoreStats, Txn, TxnMark, TxnOwner,
};
use ooxml_tools::Meanwhile;
use std::marker::PhantomData;
use std::sync::Arc;

/// Who made a version, as a format is told it ([`Holds::noted`]).
#[derive(Debug, Clone, Copy)]
pub struct StepBy<'a> {
    /// The caller's key (`None`: the session's own).
    pub caller: Option<&'a str>,
    /// The tool that made it (`None`: none named).
    pub tool: Option<&'a str>,
    /// An undo, redo or a step group's revert, which brings back a state kept.
    pub history: bool,
}

/// What holds a format's store of the model `D`, told as the store's versions are made.
pub trait Holds<D: DocModel>: Send + Sync + 'static {
    /// The store.
    fn store(&self) -> &Store<D>;

    /// The package of a version of the model.
    fn package(doc: &D) -> &Package;

    /// The version `version` was made from `previous` by `by` (another editor's version
    /// come in, an undo or redo, a step group's revert), changing the document. Nothing by
    /// default.
    ///
    /// # Errors
    /// The format's, when it cannot read the versions.
    fn noted(&self, previous: &Snapshot<D>, version: &Snapshot<D>, by: StepBy<'_>) -> Result<(), SessionError> {
        let _ = (previous, version, by);
        Ok(())
    }

    /// The version `seq` ended a step whose state is numbered `number`. Nothing by default.
    fn numbered(&self, seq: u64, number: u64) {
        let _ = (seq, number);
    }
}

impl Holds<PackageDoc> for Store<PackageDoc> {
    fn store(&self) -> &Store<PackageDoc> {
        self
    }

    fn package(doc: &PackageDoc) -> &Package {
        doc.package()
    }
}

/// A document's versions, history and step owners, over the store `H` holds.
pub struct Stored<D: DocModel, H: Holds<D> = Store<D>> {
    held: H,
    owners: Owners,
    model: PhantomData<fn() -> D>,
}

impl<D: DocModel, H: Holds<D>> Stored<D, H> {
    /// The versions `held` holds, no step owned yet.
    #[must_use]
    pub fn new(held: H) -> Self {
        Self {
            held,
            owners: Owners::default(),
            model: PhantomData,
        }
    }

    /// What holds the store.
    #[must_use]
    pub fn held(&self) -> &H {
        &self.held
    }

    /// The store.
    #[must_use]
    pub fn store(&self) -> &Store<D> {
        self.held.store()
    }

    /// The head.
    #[must_use]
    pub fn head(&self) -> Snapshot<D> {
        self.store().snapshot()
    }

    /// `snapshot` erased.
    pub fn any(snapshot: Snapshot<D>) -> AnyVersion {
        AnyVersion::of(snapshot, H::package)
    }

    /// Another editor's version came in (a co-authoring refresh): `previous` the head it
    /// was published after, `version` it, made by `editor`; and the undos that took back the
    /// steps it touched first, each `(previous, version)`. The format told of each, and of
    /// the numbers they keep.
    ///
    /// # Errors
    /// The format's, when it cannot read the versions.
    pub fn arrived(
        &self,
        undos: &[(Snapshot<D>, Snapshot<D>)],
        made: Option<(&Snapshot<D>, &Snapshot<D>)>,
        editor: &str,
    ) -> Result<(), SessionError> {
        for (previous, version) in undos {
            self.moved(previous, version, None)?;
        }
        if let Some((previous, version)) = made {
            let by = StepBy {
                caller: Some(editor),
                tool: Some(crate::refresh::REFRESH),
                history: false,
            };
            self.held.noted(previous, version, by)?;
        }
        Ok(())
    }

    /// The state numbers of the steps its history holds (undo's and plain redo's).
    fn held_numbers(&self) -> Vec<u64> {
        let listed = self.store().listed();
        listed.steps.iter().chain(&listed.redo).map(|s| s.number).collect()
    }

    /// An undo or redo, or a step group's revert, by `caller` brought back `version` from
    /// `previous`: the format told, and the state it brings back keeps its own number.
    fn moved(&self, previous: &Snapshot<D>, version: &Snapshot<D>, caller: Option<&str>) -> Result<(), SessionError> {
        let by = StepBy {
            caller,
            tool: None,
            history: true,
        };
        self.held.noted(previous, version, by)?;
        self.held.numbered(version.seq().0, version.number());
        Ok(())
    }

    /// Up to `steps` undos or redos as `caller` (`None`: the session's own); an error stops
    /// the run, and is the run's when nothing was done yet.
    fn history_run(&self, steps: usize, caller: Option<&str>, direction: Direction) -> Result<usize, SessionError> {
        let mut done = 0;
        while done < steps {
            let outcome = match (direction, caller) {
                (Direction::Undo, None) => plain(self.store().undo()),
                (Direction::Redo, None) => plain(self.store().redo()),
                (Direction::Undo, Some(caller)) => self.own_undo(caller),
                (Direction::Redo, Some(caller)) => self.own_redo(caller),
            };
            match outcome {
                Ok(Some(outcome)) => {
                    self.moved(&outcome.previous, &outcome.version, caller)?;
                    done += 1;
                }
                Ok(None) => break,
                Err(error) if done == 0 => return Err(error),
                Err(_) => break,
            }
        }
        Ok(done)
    }

    /// Undo `caller`'s latest step when it is the document's latest; `None` when it has none.
    fn own_undo(&self, caller: &str) -> Result<Option<HistoryOutcome<D>>, SessionError> {
        let store = self.store();
        loop {
            let steps = store.steps();
            let Some(index) = steps.iter().rposition(|s| s.caller.as_deref() == Some(caller)) else {
                return Ok(None);
            };
            let (Some(step), later) = (steps.get(index), steps.get(index + 1..).unwrap_or_default()) else {
                return Ok(None);
            };
            // past another editor's later steps only (never taken back): scoped, refused
            // where entangled with one of them
            let scoped = !later.is_empty() && later.iter().all(|s| s.foreign);
            if !later.is_empty() && !scoped {
                return Err(SessionError::Entangled {
                    step: step.number,
                    later: later.iter().map(|s| s.number).collect(),
                });
            }
            let undone = if scoped {
                store.undo_step(step.step, &ooxml_store::KeySet::new())
            } else {
                store.undo_newest(step.step)
            };
            match undone {
                Ok(outcome) => {
                    self.owners.undone(OwnUndo {
                        caller: Arc::from(caller),
                        number: step.number,
                        seq: step.step.0,
                        scoped,
                    });
                    return Ok(Some(outcome));
                }
                // another commit came first: it is the later change now
                Err(StoreError::NotNext { .. }) => {}
                Err(error) => return Err(crate::store_error::<D>(error, Meanwhile::History)),
            }
        }
    }

    /// Redo `caller`'s latest own undo when it is the next redo brings back; `None` when it
    /// has none (an own undo whose redo went is let go).
    fn own_redo(&self, caller: &str) -> Result<Option<HistoryOutcome<D>>, SessionError> {
        let store = self.store();
        loop {
            let Some(undo) = self.owners.latest_undo(caller) else {
                return Ok(None);
            };
            if undo.scoped {
                // taken back past another editor's step: brought back by its step, refused
                // where entangled with what came since
                return match store.redo_step(Seq(undo.seq)) {
                    Ok(outcome) => {
                        self.owners.forget(&undo);
                        Ok(Some(outcome))
                    }
                    Err(StoreError::UnknownStep { .. }) => {
                        self.owners.forget(&undo);
                        continue;
                    }
                    Err(error) => Err(crate::store_error::<D>(error, Meanwhile::History)),
                };
            }
            let Some(next) = store.redoable().pop() else {
                // nothing of it comes back: the redo went
                self.owners.forget(&undo);
                continue;
            };
            if next.number != undo.number {
                return Err(SessionError::RedoNotNext { step: undo.number });
            }
            match store.redo_next(Seq(undo.seq)) {
                Ok(outcome) => {
                    self.owners.forget(&undo);
                    return Ok(Some(outcome));
                }
                // another undo or redo came first: look again
                Err(StoreError::NotNext { .. }) => {}
                Err(error) => return Err(crate::store_error::<D>(error, Meanwhile::History)),
            }
        }
    }
}

/// Why an attempt of a batch published nothing: an atomic batch's op failed, or the store
/// refused.
enum Refused<D: DocModel> {
    Op(usize),
    Store(Error<D>),
}

impl<D: DocModel> From<Error<D>> for Refused<D> {
    fn from(error: Error<D>) -> Self {
        Refused::Store(error)
    }
}

/// One attempt of a batch in `tx`: its ops in order, each at a savepoint of its own on the
/// version the ops before it left (a failed op taken back alone, the transaction depending on
/// what its failure rests on -- [`Txn::take_back`] --, or the attempt refused where the batch
/// is atomic).
fn prepare_batch<D: DocModel>(tx: &mut Txn<'static, D>, ops: &dyn BatchOps) -> Result<(), Refused<D>> {
    ops.attempt();
    let attempt = AttemptState::default();
    let mut current = Arc::clone(tx.base());
    for index in 0..ops.count() {
        let point = tx.savepoint();
        let lent = InOp::new(tx, Arc::clone(&current), &attempt);
        let ran = ops.run(index, &lent);
        let left = lent.into_current::<D>();
        match ran {
            OpRan::Applied => current = left.unwrap_or(current),
            OpRan::NotRun => {}
            OpRan::Failed if ops.atomic() => return Err(Refused::Op(index)),
            OpRan::Failed => tx.take_back(point),
        }
    }
    Ok(())
}

/// A plain undo's or redo's outcome; `None` when there was none to take.
fn plain<D: DocModel>(outcome: Result<HistoryOutcome<D>, Error<D>>) -> Result<Option<HistoryOutcome<D>>, SessionError> {
    match outcome {
        Ok(outcome) => Ok(Some(outcome)),
        Err(StoreError::NothingToUndo | StoreError::NothingToRedo) => Ok(None),
        Err(error) => Err(crate::store_error::<D>(error, Meanwhile::History)),
    }
}

/// A document's versions and history as the session reads any document's: [`Stored`]
/// erased.
pub trait StoredDoc: Send + Sync {
    /// The head version.
    fn head(&self) -> AnyVersion;

    /// The state numbered `number` the head reaches: the head, else the newest such state undo
    /// or redo can bring back; `None` when none is kept.
    fn numbered(&self, number: u64) -> Option<AnyVersion>;

    /// The history in state numbers: the highest given, the steps undo can reach and those
    /// redo can bring back.
    fn numbers(&self) -> Numbers;

    /// The highest state number given: the next step's is one more.
    fn highest(&self) -> u64;

    /// Go on with the state numbers a host gave the document before it was opened again
    /// ([`crate::Session::continue_numbers`]).
    ///
    /// # Errors
    /// [`SessionError::Store`] when the numbers are refused.
    fn continue_numbers(&self, state: u64, highest: u64) -> Result<(), SessionError>;

    /// Who made each step, and its callers' own undos.
    fn owners(&self) -> &Owners;

    /// A step numbered `number`, ended by the version `seq`, was made by `owner` (`None`:
    /// none, a change from outside the tools): kept as its owner, the format told its number.
    fn stepped(&self, seq: u64, number: u64, owner: Option<StepOwner>);

    /// Another editor's version `seq`, numbered `number`, came in, made by `owner`: noted
    /// as [`StoredDoc::stepped`] notes a step, but no caller's own undo is let go (it drops
    /// no redo).
    fn arrived_step(&self, seq: u64, number: u64, owner: StepOwner);

    /// Up to `steps` undos or redos as `caller` (`None`: the session's own; a named caller,
    /// its own): how many were done.
    ///
    /// # Errors
    /// [`SessionError::Entangled`] or [`SessionError::RedoNotNext`] when nothing was done
    /// yet; the store's.
    fn history(&self, steps: usize, caller: Option<&str>, direction: Direction) -> Result<usize, SessionError>;

    /// Open `owner`'s step group: the mark it opened at.
    ///
    /// # Errors
    /// The store's when `owner`'s group is open already.
    fn open_group(&self, owner: TxnOwner) -> Result<TxnMark, SessionError>;

    /// Where `owner`'s open group is now.
    ///
    /// # Errors
    /// The store's without an open group.
    fn group_mark(&self, owner: TxnOwner) -> Result<TxnMark, SessionError>;

    /// Take back what `owner`'s open group did after `mark`, by `caller`, as a new version
    /// (its number; `None` when nothing was done since).
    ///
    /// # Errors
    /// The store's without an open group; the format's.
    fn revert_group(&self, owner: TxnOwner, mark: TxnMark, caller: Option<&str>) -> Result<Option<u64>, SessionError>;

    /// Commit `owner`'s open group as `caller`: the version it published and its state
    /// number, `None` when it changed nothing.
    ///
    /// # Errors
    /// The store's without an open group.
    fn commit_group(&self, owner: TxnOwner, caller: &Caller) -> Result<Option<(u64, u64)>, SessionError>;

    /// Let `owner`'s open group go, what it did taken back; whether it was open.
    fn drop_group(&self, owner: TxnOwner) -> bool;

    /// The version an op of a host's batch reads in `txn` (the one the ops before it left),
    /// where `txn` is a transaction of this document's model.
    fn version_in(&self, txn: &dyn OpTxn) -> Option<AnyVersion>;

    /// Run a host's batch of one document as `caller` ([`crate::batch_run`]): its ops in one
    /// transaction, prepared again on a conflict, committed once; what it published.
    ///
    /// # Errors
    /// [`BatchRefused::Op`] when an atomic batch's op failed; the store's.
    fn run_batch(&self, caller: &Caller, ops: &dyn BatchOps) -> Result<BatchCommitted, BatchRefused>;

    /// Run one op of a host's batch over several documents (`run`) in the transaction
    /// `owner` holds open on this one (its group), on the version its ops so far left; a
    /// failed op's changes taken back alone; `attempt` what the format keeps across the
    /// batch's ops on this document. How it ran.
    ///
    /// # Errors
    /// The store's without an open group.
    fn in_group(
        &self,
        owner: TxnOwner,
        attempt: &AttemptState,
        run: &mut dyn FnMut(&dyn OpTxn) -> OpRan,
    ) -> Result<OpRan, SessionError>;

    /// The store's counters.
    fn stats(&self) -> StoreStats;

    /// How long the store's commit lock was held and waited for.
    fn lock_times(&self) -> LockTimes;
}

impl<D: DocModel, H: Holds<D>> StoredDoc for Stored<D, H> {
    fn head(&self) -> AnyVersion {
        Self::any(Stored::head(self))
    }

    fn numbered(&self, number: u64) -> Option<AnyVersion> {
        let head = Stored::head(self);
        let found = if head.number() == number {
            Some(head)
        } else {
            self.store().numbered(number)
        };
        found.map(Self::any)
    }

    fn numbers(&self) -> Numbers {
        numbers_of(self.store().listed())
    }

    fn highest(&self) -> u64 {
        self.store().highest()
    }

    fn continue_numbers(&self, state: u64, highest: u64) -> Result<(), SessionError> {
        self.store()
            .continue_numbers(state, highest)
            .map_err(|e| StoreError::<String, crate::ModelError>::Counters(e).into())
    }

    fn owners(&self) -> &Owners {
        &self.owners
    }

    fn stepped(&self, seq: u64, number: u64, owner: Option<StepOwner>) {
        self.held.numbered(seq, number);
        self.owners.stepped(number, owner, || self.held_numbers());
    }

    fn arrived_step(&self, seq: u64, number: u64, owner: StepOwner) {
        self.held.numbered(seq, number);
        self.owners.arrived(number, owner);
    }

    fn history(&self, steps: usize, caller: Option<&str>, direction: Direction) -> Result<usize, SessionError> {
        self.history_run(steps, caller, direction)
    }

    fn open_group(&self, owner: TxnOwner) -> Result<TxnMark, SessionError> {
        let store = self.store();
        store.open_txn(owner, &D::Setup::default())?;
        Ok(store.txn_start())
    }

    fn group_mark(&self, owner: TxnOwner) -> Result<TxnMark, SessionError> {
        Ok(self.store().txn_mark(owner)?)
    }

    fn revert_group(&self, owner: TxnOwner, mark: TxnMark, caller: Option<&str>) -> Result<Option<u64>, SessionError> {
        let found = self.store().txn_revert(owner, mark)?;
        if let Some((previous, version)) = &found {
            // what the revert removed and brought back, as an undo notes it
            self.moved(previous, version, caller)?;
        }
        Ok(found.map(|(_, version)| version.seq().0))
    }

    fn commit_group(&self, owner: TxnOwner, caller: &Caller) -> Result<Option<(u64, u64)>, SessionError> {
        let done = self.store().commit_txn(owner, caller)?;
        Ok(done.changed.then(|| (done.version.seq().0, done.version.number())))
    }

    fn drop_group(&self, owner: TxnOwner) -> bool {
        self.store().drop_txn(owner)
    }

    fn version_in(&self, txn: &dyn OpTxn) -> Option<AnyVersion> {
        txn.current::<D>().map(Self::any)
    }

    fn run_batch(&self, caller: &Caller, ops: &dyn BatchOps) -> Result<BatchCommitted, BatchRefused> {
        let committed = self
            .store()
            .run_txn(caller, &D::Setup::default(), |tx| prepare_batch(tx, ops));
        match committed {
            Ok(done) => Ok(BatchCommitted {
                previous: done.previous.filter(|_| done.changed).map(Self::any),
                version: Self::any(done.version),
                attempts: done.attempts,
                exclusive: done.exclusive,
                timing: done.timing,
            }),
            Err(Refused::Op(index)) => Err(BatchRefused::Op(index)),
            Err(Refused::Store(error)) => Err(BatchRefused::Store(crate::store_error::<D>(error, Meanwhile::Edit))),
        }
    }

    fn in_group(
        &self,
        owner: TxnOwner,
        attempt: &AttemptState,
        run: &mut dyn FnMut(&dyn OpTxn) -> OpRan,
    ) -> Result<OpRan, SessionError> {
        let store = self.store();
        let current = store
            .txn_current(owner)
            .ok_or(StoreError::<D::Key, D::Error>::NotHeld)?;
        let ran = store.in_txn(owner, |tx| {
            let point = tx.savepoint();
            let ran = run(&InOp::new(tx, current, attempt));
            if ran == OpRan::Failed {
                tx.take_back(point);
            }
            ran
        })?;
        Ok(ran.out)
    }

    fn stats(&self) -> StoreStats {
        self.store().stats()
    }

    fn lock_times(&self) -> LockTimes {
        self.store().lock_times()
    }
}
