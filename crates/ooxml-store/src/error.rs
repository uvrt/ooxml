//! Owns: the store's errors ([`StoreError`], [`CountersError`]).
//!
//! Must not: map to tool errors; the tool layer does that (design section 10).

use crate::{DocModel, IdSeq, KeySet, Seq};
use std::fmt::Debug;

/// The store's error for a model `D`.
pub type Error<D> = StoreError<<D as DocModel>::Key, <D as DocModel>::Error>;

/// Why the store refused or could not do something. Nothing changed when it is returned.
#[derive(Debug, thiserror::Error)]
pub enum StoreError<K: Ord + Debug, E: std::error::Error + 'static> {
    /// A change could not be applied or inverted.
    #[error("a change could not be applied: {0}")]
    Model(#[source] E),
    /// Undo was asked for with no step to undo.
    #[error("there is no step to undo")]
    NothingToUndo,
    /// Redo was asked for with no undone step.
    #[error("there is no undone step to redo")]
    NothingToRedo,
    /// No step in the history made this version.
    #[error("no step in the history made version {step}")]
    UnknownStep {
        /// The version asked for.
        step: Seq,
    },
    /// The step cannot be undone or redone on its own: what it wrote was read or written
    /// since, by a later step or another undo, or is shared by every scope.
    #[error("the step that made version {step} is entangled with later changes on {overlap:?}")]
    Entangled {
        /// The step's version.
        step: Seq,
        /// The keys it wrote that are in the way.
        overlap: KeySet<K>,
    },
    /// Another editor's commit ([`crate::Caller::foreign`]) changed keys the transaction
    /// depends on since it was prepared: refused, not prepared again, so the caller reads
    /// them again and decides. Also an undo or redo entangled with such a commit.
    #[error("{editor} changed {overlap:?} (version {step}) since this was prepared")]
    ForeignChange {
        /// The other editor's version.
        step: Seq,
        /// Who made it.
        editor: std::sync::Arc<str>,
        /// The keys in the way.
        overlap: KeySet<K>,
    },
    /// The version a commit from another editor was made on is not one the store keeps
    /// ([`crate::Store::receive`]).
    #[error("version {seq} is not kept")]
    NotKept {
        /// The version asked for.
        seq: Seq,
    },
    /// The step asked for is no longer the one undo or redo would take next: another
    /// commit, undo or redo came first ([`crate::Store::undo_newest`]).
    #[error("the step that made version {step} is not the next to take any more")]
    NotNext {
        /// The step's version.
        step: Seq,
    },
    /// A transaction is held open (a host's batch, [`crate::Store::open_txn`]): it is
    /// committed or dropped before an undo or redo runs. (Said as a host's step group.)
    #[error("a step group is open")]
    GroupOpen,
    /// The owner holds a transaction open already ([`crate::Store::open_txn`]): one at a
    /// time per owner, which commits or drops it before it opens another.
    #[error("this owner holds a transaction on the document open already")]
    TxnHeld,
    /// The owner holds no transaction open (never opened, committed or dropped), or the mark
    /// is not one of the open one's.
    #[error("no transaction is held open by this owner")]
    NotHeld,
    /// A command runs in the held transaction already: one at a time.
    #[error("a command runs in the held transaction already")]
    TxnBusy,
    /// The held transaction could not be committed: what it was prepared on moved, which the
    /// gate it holds rules out (a bug, never the caller's); nothing changed.
    #[error("the held transaction's snapshot moved before its commit")]
    HeldMoved,
    /// An id sequence or the reopen state was refused.
    #[error(transparent)]
    Counters(#[from] CountersError),
}

/// Why reopen state or an id sequence was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CountersError {
    /// Not a state this store wrote.
    #[error("not a store state of form ooxml-store.1: {0:?}")]
    Form(String),
    /// The saved version is after the last version given.
    #[error("the saved version {version} is after the last version given, {last}")]
    VersionAfterLast {
        /// The saved version.
        version: Seq,
        /// The last version given.
        last: Seq,
    },
    /// A sequence has given every position.
    #[error("id sequence {} has given every position", .0 .0)]
    Exhausted(IdSeq),
    /// The state number to go on from is above the highest number given.
    #[error("the state number {state} is above the highest number given, {highest}")]
    StateAboveHighest {
        /// The state number.
        state: u64,
        /// The highest number given.
        highest: u64,
    },
    /// The store numbered a state already: its numbers go on from its own.
    #[error("the store numbered a state already")]
    AlreadyNumbered,
}
