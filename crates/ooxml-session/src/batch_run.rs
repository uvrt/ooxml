//! Owns: a host's batch of changing calls as a document's store runs it (design phase 3,
//! section 4): one transaction and one commit, prepared optimistically and prepared again on
//! a conflict ([`ooxml_store::Store::run_txn`]: up to the store's optimistic tries, then
//! behind its gate) -- each op run by the host ([`BatchOps`]) in the transaction handed on
//! ([`OpTxn`]) at a savepoint of its own, a failed op taken back alone, or the whole batch
//! refused where it is atomic -- and one op of a batch over several documents run in the
//! transaction its owner holds open ([`crate::StoredDoc::in_group`]).
//!
//! Must not: run a tool (the host does, [`BatchOps::run`]), or keep anything an attempt did
//! past it: an attempt that conflicts is forgotten ([`BatchOps::attempt`]), and the host
//! applies its ops' effects once the batch is published.

use crate::format::AnyVersion;
use ooxml_store::{CommitTiming, OpTxn};

/// The ops of a host's batch on one document, as its store runs them: once per attempt, in
/// order, each in the batch's transaction.
pub trait BatchOps: Sync {
    /// An attempt begins: what the ops gathered in an earlier one (their results, their
    /// effects, the refs they were to define) is dropped.
    fn attempt(&self);

    /// How many ops run in the transaction.
    fn count(&self) -> usize;

    /// Whether a failed op refuses the whole batch (nothing published) rather than being
    /// taken back alone.
    fn atomic(&self) -> bool;

    /// Run op `index` in `txn` (whose version is the one the ops before it left), keeping its
    /// result for this attempt.
    fn run(&self, index: usize, txn: &dyn OpTxn) -> OpRan;
}

/// How one op of an attempt ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpRan {
    /// Its changes are kept.
    Applied,
    /// It failed: taken back alone (its changes dropped), or the batch refused if atomic.
    Failed,
    /// Not run (the batch's deadline passed): it changed nothing.
    NotRun,
}

/// A batch's commit.
#[derive(Clone)]
pub struct BatchCommitted {
    /// The version published, or the one the batch ran on where it changed nothing.
    pub version: AnyVersion,
    /// The version it was published after (the head then, which may be newer than the one it
    /// was prepared on); `None` where it changed nothing.
    pub previous: Option<AnyVersion>,
    /// How many times it was prepared.
    pub attempts: u32,
    /// Whether it ran behind the document's gate after conflicts.
    pub exclusive: bool,
    /// Where its time went.
    pub timing: CommitTiming,
}

/// Why a batch published nothing.
#[derive(Debug)]
pub enum BatchRefused {
    /// The atomic batch's op `index` failed.
    Op(usize),
    /// The store's refusal.
    Store(crate::SessionError),
}
