//! Owns: a transaction one owner holds open across calls ([`Store::open_txn`]): a host's
//! batch of commands made one transaction, with a savepoint to return to per mark, the
//! version each command left for the owner to read on (numbered as the commit will number
//! it), and one commit at the end ([`Store::commit_txn`]) -- or none ([`Store::drop_txn`]).
//!
//! The holder keeps the gate closed from open to commit or drop: other writers prepare on
//! the head meanwhile and publish after it; undo and redo are refused. Its commit therefore
//! lands on the snapshot it was begun on. One transaction is held at a time: another owner's
//! open waits until it is let go; the same owner's is refused.
//!
//! Must not: publish a version before the commit, or let the version the owner reads be seen
//! by anyone else (a reader that is not the owner reads the head); hold the holder's lock while
//! a command prepares or while the gate is waited for.

use crate::sync::{lock, wait};
use crate::{Caller, Committed, DocModel, Error, KeySet, Savepoint, Snapshot, Store, StoreError, Txn};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::Arc;

/// Who holds a transaction open: a value the host gives (one per thread of its, say).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TxnOwner(u64);

impl TxnOwner {
    /// The owner named `id`.
    #[must_use]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }
}

/// A point in a held transaction to return to ([`Store::txn_mark`], [`Store::txn_revert`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxnMark(usize);

/// What a command run in a held transaction did ([`Store::in_txn`]).
#[derive(Debug)]
pub struct InTxn<D: DocModel, R> {
    /// What the command returned.
    pub out: R,
    /// The version the owner read before it.
    pub before: Snapshot<D>,
    /// The version the owner reads after it: a new one where it changed something.
    pub after: Snapshot<D>,
    /// Whether it changed the document.
    pub changed: bool,
}

/// A revert's versions: the one the owner read before it and the one it reads after.
pub type Reverted<D> = (Snapshot<D>, Snapshot<D>);

/// The transaction held open and what it keeps beside it.
pub(crate) struct HeldTxn<D: DocModel> {
    owner: TxnOwner,
    /// `None` while a command runs in it (or while it is being opened).
    txn: Option<Txn<'static, D>>,
    /// Per mark, the point and the version the owner read there; the first is the open.
    marks: Vec<(Savepoint<D>, Snapshot<D>)>,
    /// The version the owner reads now.
    current: Snapshot<D>,
    /// The state number its versions carry: the one its commit will be given.
    number: u64,
}

impl<D: DocModel> Store<D> {
    /// Hold a transaction open on the head for `owner`, its working version set up with
    /// `setup`: the gate is closed until it is committed or dropped. Waits while another
    /// owner holds one (and for the gate); the version begun on.
    ///
    /// # Errors
    /// [`StoreError::TxnHeld`] when `owner` holds one already.
    pub fn open_txn(&self, owner: TxnOwner, setup: &D::Setup) -> Result<Snapshot<D>, Error<D>> {
        let mut holder = lock(&self.holder);
        loop {
            match &*holder {
                None => break,
                Some(held) if held.owner == owner => return Err(StoreError::TxnHeld),
                Some(_) => holder = wait(&self.released, holder),
            }
        }
        // the slot is taken before the gate is waited for, without the holder's lock held
        let base = self.snapshot();
        *holder = Some(HeldTxn {
            owner,
            txn: None,
            marks: Vec::new(),
            current: Arc::clone(&base),
            number: 0,
        });
        drop(holder);
        let mut state = self.close_gate();
        // under the commit lock with the gate closed: nothing publishes after this head
        state.held = true;
        let number = state.numbered.saturating_add(1);
        let tx = self.begin(setup);
        drop(state);
        let base = Arc::clone(tx.base());
        let mut holder = lock(&self.holder);
        match holder.as_mut().filter(|held| held.owner == owner) {
            Some(held) => {
                held.marks.push((tx.savepoint(), Arc::clone(&base)));
                held.current = Arc::clone(&base);
                held.number = number;
                held.txn = Some(tx);
                Ok(base)
            }
            // (never: a slot being opened is not dropped, [`Store::drop_txn`])
            None => {
                self.open_gate(|state| state.held = false);
                Err(StoreError::NotHeld)
            }
        }
    }

    /// Run `command` in `owner`'s held transaction (one at a time): what it returned, and
    /// the versions the owner read before and after it. A panic in it returns the
    /// transaction to where it was before it and goes on.
    ///
    /// # Errors
    /// [`StoreError::NotHeld`]; [`StoreError::TxnBusy`] when a command runs in it already.
    pub fn in_txn<R>(
        &self,
        owner: TxnOwner,
        command: impl FnOnce(&mut Txn<'static, D>) -> R,
    ) -> Result<InTxn<D, R>, Error<D>> {
        let (mut tx, before) = {
            let mut holder = lock(&self.holder);
            let held = holder
                .as_mut()
                .filter(|held| held.owner == owner)
                .ok_or(StoreError::NotHeld)?;
            (held.txn.take().ok_or(StoreError::TxnBusy)?, Arc::clone(&held.current))
        };
        let point = tx.savepoint();
        let ran = catch_unwind(AssertUnwindSafe(|| command(&mut tx)));
        let changed = tx.len() != point.len();
        let out = match ran {
            Ok(out) => out,
            Err(panic) => {
                tx.rollback(point);
                // (let go meanwhile: nothing to put back)
                let _ = self.put_back(owner, tx, None);
                resume_unwind(panic)
            }
        };
        let after = if changed {
            // a number of its own, never given again: a reading kept by version number (the
            // readings of a document's versions) never mistakes one for another
            let seq = self.draw_seq();
            let number = lock(&self.holder)
                .as_ref()
                .filter(|held| held.owner == owner)
                .map_or(before.number(), |held| held.number);
            let doc = tx.doc().clone();
            doc.settle();
            Arc::new(tx.base().succeed(seq, number, doc, tx.writes()))
        } else {
            Arc::clone(&before)
        };
        self.put_back(owner, tx, Some(Arc::clone(&after)))?;
        Ok(InTxn {
            out,
            before,
            after,
            changed,
        })
    }

    /// `tx` back in `owner`'s slot, the owner reading `current` now (where given).
    fn put_back(&self, owner: TxnOwner, tx: Txn<'static, D>, current: Option<Snapshot<D>>) -> Result<(), Error<D>> {
        let mut holder = lock(&self.holder);
        // dropped meanwhile: its work goes with it
        let held = holder
            .as_mut()
            .filter(|held| held.owner == owner)
            .ok_or(StoreError::NotHeld)?;
        held.txn = Some(tx);
        if let Some(current) = current {
            held.current = current;
        }
        Ok(())
    }

    /// The version `owner` reads in the transaction it holds (while a command runs in it,
    /// the one it runs on); `None` when it holds none.
    pub fn txn_current(&self, owner: TxnOwner) -> Option<Snapshot<D>> {
        let holder = lock(&self.holder);
        let held = holder
            .as_ref()
            .filter(|held| held.owner == owner && !held.marks.is_empty())?;
        Some(Arc::clone(&held.current))
    }

    /// The version `owner`'s held transaction was begun on; `None` when it holds none.
    pub fn txn_base(&self, owner: TxnOwner) -> Option<Snapshot<D>> {
        let holder = lock(&self.holder);
        let held = holder.as_ref().filter(|held| held.owner == owner)?;
        held.marks.first().map(|(_, version)| Arc::clone(version))
    }

    /// Whether `owner` holds a transaction open.
    pub fn holds(&self, owner: TxnOwner) -> bool {
        lock(&self.holder).as_ref().is_some_and(|held| held.owner == owner)
    }

    /// Whether any owner holds a transaction open.
    pub fn txn_held(&self) -> bool {
        lock(&self.holder).is_some()
    }

    /// A point to return to in `owner`'s held transaction: where it is now.
    ///
    /// # Errors
    /// [`StoreError::NotHeld`]; [`StoreError::TxnBusy`].
    pub fn txn_mark(&self, owner: TxnOwner) -> Result<TxnMark, Error<D>> {
        let mut holder = lock(&self.holder);
        let held = holder
            .as_mut()
            .filter(|held| held.owner == owner)
            .ok_or(StoreError::NotHeld)?;
        let tx = held.txn.as_ref().ok_or(StoreError::TxnBusy)?;
        let point = tx.savepoint();
        let current = Arc::clone(&held.current);
        held.marks.push((point, current));
        Ok(TxnMark(held.marks.len() - 1))
    }

    /// The mark the held transaction of `owner` was opened at.
    pub fn txn_start(&self) -> TxnMark {
        TxnMark(0)
    }

    /// Return `owner`'s held transaction to `mark`: the changes after it dropped (the ids
    /// drawn after it stay drawn), the marks after it let go. The version the owner read
    /// before and the one it reads now -- the content and state number it read at `mark`, as
    /// a version of its own (a number never given before, as an undo's); `None` when nothing
    /// changed since `mark`.
    ///
    /// # Errors
    /// [`StoreError::NotHeld`] (also for a mark let go); [`StoreError::TxnBusy`].
    pub fn txn_revert(&self, owner: TxnOwner, mark: TxnMark) -> Result<Option<Reverted<D>>, Error<D>> {
        let mut holder = lock(&self.holder);
        let held = holder
            .as_mut()
            .filter(|held| held.owner == owner)
            .ok_or(StoreError::NotHeld)?;
        let tx = held.txn.as_mut().ok_or(StoreError::TxnBusy)?;
        let (point, version) = held.marks.get(mark.0).ok_or(StoreError::NotHeld)?;
        if tx.len() == point.len() && Arc::ptr_eq(version, &held.current) {
            return Ok(None);
        }
        let (point, at) = (point.clone(), Arc::clone(version));
        tx.rollback(point);
        held.marks.truncate(mark.0 + 1);
        drop(holder);
        let version = Arc::new(at.succeed(self.draw_seq(), at.number(), at.doc().clone(), &KeySet::new()));
        let mut holder = lock(&self.holder);
        let held = holder
            .as_mut()
            .filter(|held| held.owner == owner)
            .ok_or(StoreError::NotHeld)?;
        if let Some(marked) = held.marks.get_mut(mark.0) {
            // the version read at the mark now: a revert to it again finds nothing since
            marked.1 = Arc::clone(&version);
        }
        let before = std::mem::replace(&mut held.current, Arc::clone(&version));
        Ok(Some((before, version)))
    }

    /// Commit `owner`'s held transaction as `caller` -- one commit and one history step, on
    /// the version it was begun on (the gate held it there) -- and let it go, the gate open
    /// again. The version published is the one the owner read last, under its number (the
    /// same content: a reading kept by that number stays right). Its result
    /// ([`Committed::changed`]: whether it changed anything).
    ///
    /// # Errors
    /// [`StoreError::NotHeld`]; [`StoreError::TxnBusy`]; [`StoreError::HeldMoved`].
    pub fn commit_txn(&self, owner: TxnOwner, caller: &Caller) -> Result<Committed<D, ()>, Error<D>> {
        let start = self.clock.monotonic();
        let (tx, at) = {
            let mut holder = lock(&self.holder);
            let held = holder
                .as_mut()
                .filter(|held| held.owner == owner)
                .ok_or(StoreError::NotHeld)?;
            let tx = held.txn.take().ok_or(StoreError::TxnBusy)?;
            (tx, held.current.seq())
        };
        let done = self.commit_gated(caller, tx, Some(at), start);
        self.release(owner);
        done.ok_or(StoreError::HeldMoved)
    }

    /// Let `owner`'s held transaction go uncommitted, the gate open again; whether it held
    /// one. Nothing it did is published (the ids it drew stay drawn).
    pub fn drop_txn(&self, owner: TxnOwner) -> bool {
        // (one being opened is not: its opener closes the gate yet)
        let holds = lock(&self.holder)
            .as_ref()
            .is_some_and(|held| held.owner == owner && !held.marks.is_empty());
        if holds {
            self.release(owner);
        }
        holds
    }

    /// The gate opened, undo and redo let through again, and the slot freed.
    fn release(&self, owner: TxnOwner) {
        self.open_gate(|state| state.held = false);
        self.let_go(owner);
    }

    /// `owner`'s slot freed; the next owner waiting goes on.
    fn let_go(&self, owner: TxnOwner) {
        let mut holder = lock(&self.holder);
        if holder.as_ref().is_some_and(|held| held.owner == owner) {
            *holder = None;
        }
        drop(holder);
        self.released.notify_all();
    }
}
