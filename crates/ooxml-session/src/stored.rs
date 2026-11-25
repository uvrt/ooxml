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

use crate::format::AnyVersion;
use crate::history::{numbers_of, Direction, Numbers};
use crate::model::{PackageDoc, PartChange, PartContent};
use crate::owners::{OwnUndo, Owners, StepOwner};
use crate::update::{PartUpdate, Parts};
use crate::SessionError;
use ooxml_opc::Package;
use ooxml_store::{
    Caller, Command, DocModel, Error, HistoryOutcome, LockTimes, Seq, Snapshot, Store, StoreError, StoreStats, TxnMark,
    TxnOwner,
};
use ooxml_xml::Tree;
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

    /// The change that puts `tree` as `part` (a test's update of parts).
    fn put_tree(part: String, tree: Tree) -> D::Change;

    /// The change that puts `bytes` as `part`.
    fn put_bytes(part: String, bytes: Arc<[u8]>) -> D::Change;

    /// The change that removes `part`.
    fn remove(part: String) -> D::Change;

    /// What the put trees add to the version's ledger, if anything.
    fn ledger(doc: &D, trees: &[&Tree]) -> Option<D::Change>;

    /// The version `version` was made from `previous` by `by` (a test's update, an undo or
    /// redo, a step group's revert), changing the document. Nothing by default.
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

    fn put_tree(part: String, tree: Tree) -> PartChange {
        PartChange::Put {
            part,
            content: PartContent::Tree(tree),
        }
    }

    fn put_bytes(part: String, bytes: Arc<[u8]>) -> PartChange {
        PartChange::Put {
            part,
            content: PartContent::Bytes(bytes),
        }
    }

    fn remove(part: String) -> PartChange {
        PartChange::Remove { part }
    }

    fn ledger(_doc: &PackageDoc, _trees: &[&Tree]) -> Option<PartChange> {
        None
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

    /// The head, or for the thread whose step group holds the document the version its ops
    /// left ([`crate::group`]).
    #[must_use]
    pub fn head(&self) -> Snapshot<D> {
        let store = self.store();
        store
            .txn_current(crate::group::this_thread())
            .unwrap_or_else(|| store.snapshot())
    }

    /// The owner of the step group this thread holds on the document, if it holds one: its
    /// calls' edits run in the group's transaction.
    #[must_use]
    pub fn batch_owner(&self) -> Option<TxnOwner> {
        let owner = crate::group::this_thread();
        self.store().holds(owner).then_some(owner)
    }

    /// `snapshot` erased.
    pub fn any(snapshot: Snapshot<D>) -> AnyVersion {
        AnyVersion::of(snapshot, H::package)
    }

    /// Commit a test's update of parts as `caller`, made by `tool`: in this thread's step
    /// group where it holds one, at a savepoint (a commit of its own would wait for the gate
    /// the group holds), else committed. The version it leaves, whether it changed anything,
    /// how many times it was prepared, and whether it ran in the group.
    ///
    /// # Errors
    /// The update's or the store's.
    pub fn update(
        &self,
        caller: &Caller,
        tool: Option<&str>,
        update: &PartUpdate<'_>,
    ) -> Result<(Snapshot<D>, bool, u32, bool), SessionError> {
        let store = self.store();
        let update = &Parts::<D, H> {
            update,
            model: PhantomData,
        };
        let (version, previous, attempts, batch) = match self.batch_owner() {
            Some(owner) => {
                let ran = store.in_txn(owner, |tx| {
                    let point = tx.savepoint();
                    let done = update.validate(tx.doc()).and_then(|()| update.prepare(tx));
                    if done.is_err() {
                        tx.rollback(point);
                    }
                    done
                })?;
                ran.out?;
                (ran.after, ran.changed.then_some(ran.before), 1, true)
            }
            None => {
                let done = store.run(caller, update)?;
                let previous = done.previous.filter(|_| done.changed);
                (done.version, previous, done.attempts, false)
            }
        };
        if let Some(previous) = &previous {
            let by = StepBy {
                caller: caller.name(),
                tool,
                history: false,
            };
            self.held.noted(previous, &version, by)?;
        }
        Ok((version, previous.is_some(), attempts, batch))
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
            if !later.is_empty() {
                return Err(SessionError::Entangled {
                    step: step.number,
                    later: later.iter().map(|s| s.number).collect(),
                });
            }
            match store.undo_newest(step.step) {
                Ok(outcome) => {
                    self.owners.undone(OwnUndo {
                        caller: Arc::from(caller),
                        number: step.number,
                        seq: step.step.0,
                    });
                    return Ok(Some(outcome));
                }
                // another commit came first: it is the later change now
                Err(StoreError::NotNext { .. }) => {}
                Err(error) => return Err(error.into()),
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
                Err(error) => return Err(error.into()),
            }
        }
    }
}

/// A plain undo's or redo's outcome; `None` when there was none to take.
fn plain<D: DocModel>(outcome: Result<HistoryOutcome<D>, Error<D>>) -> Result<Option<HistoryOutcome<D>>, SessionError> {
    match outcome {
        Ok(outcome) => Ok(Some(outcome)),
        Err(StoreError::NothingToUndo | StoreError::NothingToRedo) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// A document's versions and history as the session reads any document's: [`Stored`]
/// erased.
pub trait StoredDoc: Send + Sync {
    /// The head version; for the thread whose step group holds the document, the version
    /// its ops left.
    fn head(&self) -> AnyVersion;

    /// The state numbered `number` the head reaches: this thread's group's version or the
    /// head, else the newest such state undo or redo can bring back; `None` when none is kept.
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
        // this thread's group's version first: its ops' state, numbered as it will be
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

    fn stats(&self) -> StoreStats {
        self.store().stats()
    }

    fn lock_times(&self) -> LockTimes {
        self.store().lock_times()
    }
}
