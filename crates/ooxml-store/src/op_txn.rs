//! Owns: a transaction handed on erased ([`OpTxn`], made by [`InOp`]): a host's batch runs
//! each op in the batch's one transaction, through code that does not name the document's
//! model (a host, a session), down to the format's code, which names it and takes the
//! transaction back ([`OpTxn::with`]) with the version the op reads -- the one the batch's
//! ops so far left, never published ([`crate::Txn::version_here`]) -- and what the format
//! keeps across the ops of one attempt ([`OpTxn::attempt`]: a Word document's tracking mode
//! an op set, for the ops after it).
//!
//! Must not: commit, or outlive the op it is handed to (it borrows the transaction).

use crate::{DocModel, Snapshot, Txn};
use std::any::Any;
use std::sync::{Mutex, PoisonError};

/// A transaction for one op, erased: the format's code takes it back by its model
/// ([`OpTxn::with`]).
pub trait OpTxn: Sync {
    /// What the format keeps across the ops of this attempt of the batch (empty at its start;
    /// dropped with the attempt).
    fn attempt(&self) -> &AttemptState;

    /// The version the op reads now, erased.
    fn current_erased(&self) -> &Mutex<Box<dyn Any + Send + Sync>>;

    /// Whether the batch's transaction is prepared behind the document's gate (other writers
    /// wait for it: [`Txn::gated`]), else optimistically.
    fn gated(&self) -> bool;

    /// `f` handed the transaction, erased (held for `f` alone: what `f` runs may read the
    /// version the op reads, [`OpTxn::current`], but not take the transaction again).
    fn erased(&self, f: &mut dyn FnMut(&mut (dyn Any + Send + 'static)));
}

impl dyn OpTxn + '_ {
    /// `f` run on the transaction and the version the op reads now (which `f` moves on where
    /// it made one), where they are of the model `D`; `None` where they are another's.
    pub fn with<D: DocModel, R>(&self, f: impl FnOnce(&mut Txn<'static, D>, &mut Snapshot<D>) -> R) -> Option<R> {
        let mut current = self.current::<D>()?;
        let mut f = Some(f);
        let mut out = None;
        self.erased(&mut |tx| {
            if let (Some(tx), Some(f)) = (tx.downcast_mut::<Txn<'static, D>>(), f.take()) {
                out = Some(f(tx, &mut current));
            }
        });
        if out.is_some() {
            *self.current_erased().lock().unwrap_or_else(PoisonError::into_inner) = Box::new(current);
        }
        out
    }

    /// The version the op reads now, where it is of the model `D`.
    pub fn current<D: DocModel>(&self) -> Option<Snapshot<D>> {
        let current = self.current_erased().lock().unwrap_or_else(PoisonError::into_inner);
        current.downcast_ref::<Snapshot<D>>().cloned()
    }
}

/// A transaction lent to one op, with the version the op reads.
pub struct InOp<'t> {
    tx: Mutex<&'t mut (dyn Any + Send + 'static)>,
    gated: bool,
    current: Mutex<Box<dyn Any + Send + Sync>>,
    attempt: &'t AttemptState,
}

/// What the format keeps across the ops of one attempt of a batch ([`OpTxn::attempt`]).
pub type AttemptState = Mutex<Option<Box<dyn Any + Send>>>;

impl<'t> InOp<'t> {
    /// `tx` lent to an op that reads `current` (the batch's ops so far left it), in the
    /// attempt that keeps `attempt`.
    pub fn new<D: DocModel>(tx: &'t mut Txn<'static, D>, current: Snapshot<D>, attempt: &'t AttemptState) -> Self {
        InOp {
            gated: tx.gated(),
            tx: Mutex::new(tx),
            current: Mutex::new(Box::new(current)),
            attempt,
        }
    }

    /// The version the op left for the next to read, where it is of the model `D`.
    pub fn into_current<D: DocModel>(self) -> Option<Snapshot<D>> {
        let current = self.current.into_inner().unwrap_or_else(PoisonError::into_inner);
        current.downcast::<Snapshot<D>>().ok().map(|c| *c)
    }
}

impl OpTxn for InOp<'_> {
    fn attempt(&self) -> &AttemptState {
        self.attempt
    }

    fn current_erased(&self) -> &Mutex<Box<dyn Any + Send + Sync>> {
        &self.current
    }

    fn gated(&self) -> bool {
        self.gated
    }

    fn erased(&self, f: &mut dyn FnMut(&mut (dyn Any + Send + 'static))) {
        // (a panic in an op leaves the transaction to its batch, which takes the op back)
        let mut tx = self.tx.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut **tx);
    }
}
