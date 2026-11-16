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
}
