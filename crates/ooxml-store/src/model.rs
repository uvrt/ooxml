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
    /// How one run's transaction prepares (a setting of the call it serves, not content:
    /// nothing in it is a change), given to its working version ([`DocModel::set_up`]);
    /// `()` for a model with none.
    type Setup: Default + Send + Sync + 'static;
    /// What one version changed against the version before it, by the model's own identity
    /// ([`DocModel::delta`]); `()` for a model that has none to give.
    type Delta: Send + Sync + 'static;

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

    /// Whether `key` names what no step takes back (an id sequence drawn from: an undo keeps
    /// every id given, [`DocModel::carry`]). A later step writing such a key does not
    /// entangle an earlier one with it. None by default.
    fn kept(key: &Self::Key) -> bool {
        let _ = key;
        false
    }

    /// Whether `key` names what is derived from content the writer keys as well (a summary
    /// of a part's tree, say, whose blocks a change of the tree writes): recomputed where a
    /// change is applied, so it is commutative bookkeeping, not an exclusive write. A
    /// transaction or step that read it conflicts with, or is entangled by, a writer of it;
    /// two writers of it do neither (the content keys they write decide). None by default.
    fn derived(key: &Self::Key) -> bool {
        let _ = key;
        false
    }

    /// What `after` changed against `before`: the store works it out for each version it
    /// publishes against the version it was published after, when first asked
    /// ([`crate::Version::changes`]) -- so a commit, an undo, a redo and a revert all get it
    /// from this one function.
    fn delta(before: &Self, after: &Self) -> Self::Delta;

    /// Derive now what a version keeps beside its content (its indexes), from the version
    /// before it. The store calls it on each version it publishes, so every head has them
    /// and a version's cost is that of its own change: a change applied to the head (an id
    /// drawn from it) and the next version both derive from the head's. Nothing by default.
    fn settle(&self) {}

    /// Take into `self`, an undo's or a redo's new version, what `head` (the version it is
    /// made on) holds that no step takes back: an id ledger, where an id once given is never
    /// given again (design sections 4 and 9). The store calls it on every version an undo or
    /// redo makes, before publishing it. Nothing by default.
    fn carry(&mut self, head: &Self) {
        let _ = head;
    }

    /// Add to `into` the keys of [`DocModel::written`] that `change` writes as a merge made
    /// where it is applied (an id drawn from the version it lands on, a name added to a
    /// list): a transaction that read one conflicts with it, but two that both write one
    /// do not, as their changes give the serial result in either commit order. None by
    /// default.
    fn merged(change: &Self::Change, into: &mut KeySet<Self::Key>) {
        let _ = (change, into);
    }

    /// Begin a transaction on `self`: what a version keeps of the transaction that made it
    /// (the values its stand-ins were drawn as, [`DocModel::resolve`]) is let go, so the
    /// transaction's own draws start afresh. The store calls it on a transaction's working
    /// version, on the head a commit is replayed on, and before it replays a step's changes
    /// or their inverses (undo, redo). Nothing by default.
    fn begin(&mut self) {}

    /// Set up a transaction's working version, after [`DocModel::begin`], with how its run
    /// prepares ([`crate::Store::run_with`]). It may steer only what the transaction's
    /// command decides while preparing: a commit replays the changes alone. Nothing by
    /// default.
    fn set_up(&mut self, setup: &Self::Setup) {
        let _ = setup;
    }

    /// Add to `into` the keys `change` creates under stand-ins: keys of what did not exist
    /// before, named by a value drawn where the change is applied (a new block named by its
    /// new id). They are kept apart from [`DocModel::written`] end to end: a stand-in may
    /// equal a key another transaction has just published for real, so it is never compared
    /// as a key. No other transaction can have read one on its snapshot, so they are no
    /// conflict; at commit they are resolved to the keys drawn ([`DocModel::resolve`]) and
    /// only then stamped, kept in the history and journaled. None by default.
    fn created(change: &Self::Change, into: &mut KeySet<Self::Key>) {
        let _ = (change, into);
    }

    /// The keys `created` (stand-ins, [`DocModel::created`]) name in `self`, the version a
    /// transaction made: each as the value drawn where its change was applied. None by
    /// default (a model that creates none).
    fn resolve(&self, created: &KeySet<Self::Key>) -> KeySet<Self::Key> {
        let _ = created;
        KeySet::new()
    }

    /// `change`, made on the version `past` was made on, moved to apply after `past`
    /// (design section 5: a commit's patches rebased through those committed since its
    /// snapshot). `first` says which of the two was published first: where both put
    /// something at one place, that one's comes first, whichever is moved
    /// ([`First`]). So `past` then `rebase(change, past, First::Past)` gives what `change`
    /// then `rebase(past, change, First::Change)` gives. Called only when the two
    /// transactions' keys are disjoint; an error means the two touch the same content after
    /// all, and the commit is prepared again. The default keeps the change as it is, right
    /// for changes addressed by key rather than by position.
    fn rebase(change: &Self::Change, past: &Self::Change, first: First) -> Result<Self::Change, Self::Error> {
        let _ = (past, first);
        Ok(change.clone())
    }
}

/// Which of two changes made on one version was published first, where one is moved past
/// the other ([`DocModel::rebase`]): where both put something at one place, its comes first.
/// The one rule a sequence's rebase needs to give one result either way round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum First {
    /// `past` was: a commit's change moved past one published since its snapshot.
    Past,
    /// `change` was: a change published since a commit's snapshot, carried past that
    /// commit's earlier changes (so that it stands where the commit's next change was made).
    Change,
}
