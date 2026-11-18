//! Owns: [`DocModel`], what the store needs to know of a document: its keys, its changes,
//! how a change applies and how it is undone.
//!
//! Must not: carry any format's names; the format crates implement the trait.

use crate::KeySet;
use std::fmt::Debug;

/// A document as the store keeps it: a persistent value (cloning is cheap and shares
/// structure), changed only by applying [`DocModel::Change`]s.
pub trait DocModel: Clone + Send + Sync + 'static {
    /// What a transaction reads and a change writes: a block, a part, a style, a list chain.
    type Key: Clone + Ord + Debug + Send + Sync + 'static;
    /// One primitive of a change set (a tree patch, a ledger addition, ...).
    type Change: Clone + Debug + Send + Sync + 'static;
    /// Why a change could not be applied or inverted.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Apply `change`. On an error the store discards the value, so it need not be left as
    /// it was. Applied to a version that agrees on every key the change's transaction read,
    /// the result must be what the transaction's `prepare` would have made there.
    fn apply(&mut self, change: &Self::Change) -> Result<(), Self::Error>;

    /// The change that undoes `change` when applied right after it; `self` is the version
    /// `change` applies to.
    fn inverse(&self, change: &Self::Change) -> Result<Self::Change, Self::Error>;

    /// Whether `self` and `other` hold the same content under every key of `keys` (the
    /// whole document for [`KeySet::Everything`]). Versions share structure, so pointer
    /// equality answers most of it.
    fn agrees(&self, other: &Self, keys: &KeySet<Self::Key>) -> bool;

    /// Add the keys `change` writes to `into` ([`KeySet::insert_everything`] when its extent
    /// is not known).
    fn written(change: &Self::Change, into: &mut KeySet<Self::Key>);

    /// Derive now what a version keeps beside its content (its indexes), from the version
    /// before it. The store calls it on each version it publishes, so every head has them
    /// and a version's cost is that of its own change: a change applied to the head (an id
    /// drawn from it) and the next version both derive from the head's. Nothing by default.
    fn settle(&self) {}

    /// Add to `into` the keys of [`DocModel::written`] that `change` writes as a merge made
    /// where it is applied (an id drawn from the version it lands on, a name added to a
    /// list): a transaction that read one conflicts with it, but two that both write one
    /// do not, as their changes give the serial result in either commit order. None by
    /// default.
    fn merged(change: &Self::Change, into: &mut KeySet<Self::Key>) {
        let _ = (change, into);
    }

    /// `change`, made on a version `past` had not been applied to, moved to apply after
    /// `past` (design section 5: a commit's patches rebased through those committed since
    /// its snapshot). Called only when the two transactions' keys are disjoint; an error
    /// means the two touch the same content after all, and the commit is prepared again.
    /// The default keeps the change as it is, right for changes addressed by key rather
    /// than by position.
    fn rebase(change: &Self::Change, past: &Self::Change) -> Result<Self::Change, Self::Error> {
        let _ = past;
        Ok(change.clone())
    }
}
