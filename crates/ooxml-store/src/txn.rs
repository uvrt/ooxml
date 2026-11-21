//! Owns: [`Txn`], a transaction: a snapshot, a private working version made from it, the
//! keys read, the keys written and the change set; savepoints for batches.
//!
//! Must not: publish anything; [`crate::Store`] commits a transaction.

use crate::ids::IdAllocator;
use crate::{DocModel, Error, IdSeq, KeySet, ReadPredicate, Snapshot, StoreError};
use std::sync::Arc;

/// A transaction on a snapshot. Its working version is private and cheap (versions are
/// persistent); its changes reach the document only when the store commits it.
pub struct Txn<'s, D: DocModel> {
    base: Snapshot<D>,
    work: D,
    reads: KeySet<D::Key>,
    /// What it read by condition ([`Txn::read_where`]).
    predicates: Vec<Arc<dyn ReadPredicate<D>>>,
    writes: KeySet<D::Key>,
    /// The written keys that are merges ([`DocModel::merged`]).
    merges: KeySet<D::Key>,
    /// The keys its changes create under stand-ins ([`DocModel::created`]).
    created: KeySet<D::Key>,
    changes: Vec<D::Change>,
    ids: &'s IdAllocator,
}

/// A point in a transaction to return to (a batch's failed op returns to the one taken
/// before it). Reads recorded after it are kept: a conflict on them only redoes the work.
pub struct Savepoint<D: DocModel> {
    work: D,
    writes: KeySet<D::Key>,
    merges: KeySet<D::Key>,
    created: KeySet<D::Key>,
    changes: usize,
}

impl<'s, D: DocModel> Txn<'s, D> {
    pub(crate) fn new(base: Snapshot<D>, ids: &'s IdAllocator) -> Self {
        let mut work = base.doc().clone();
        work.begin();
        Self {
            base,
            work,
            reads: KeySet::new(),
            predicates: Vec::new(),
            writes: KeySet::new(),
            merges: KeySet::new(),
            created: KeySet::new(),
            changes: Vec::new(),
            ids,
        }
    }

    /// The snapshot the transaction started from.
    pub fn base(&self) -> &Snapshot<D> {
        &self.base
    }

    /// The working version: the snapshot with this transaction's changes so far.
    pub fn doc(&self) -> &D {
        &self.work
    }

    /// Record that the transaction's changes depend on `key`.
    pub fn read(&mut self, key: D::Key) {
        self.reads.insert(key);
    }

    /// Record that the transaction depends on the whole document: it commits only if no
    /// other commit came between its snapshot and its commit.
    pub fn read_everything(&mut self) {
        self.reads.insert_everything();
    }

    /// Record that the transaction depends on every key that meets `predicate` (a find over
    /// every paragraph depends on the paragraphs that hold its text): it commits unless a key
    /// written since its snapshot meets it on the head. The keys that met it on the snapshot
    /// and that the outcome used must also be read by key ([`ReadPredicate`]'s contract).
    pub fn read_where(&mut self, predicate: Arc<dyn ReadPredicate<D>>) {
        self.predicates.push(predicate);
    }

    /// Apply `change` to the working version and add it to the change set.
    pub fn write(&mut self, change: D::Change) -> Result<(), Error<D>> {
        let mut next = self.work.clone();
        next.apply(&change).map_err(StoreError::Model)?;
        self.work = next;
        D::written(&change, &mut self.writes);
        D::merged(&change, &mut self.merges);
        D::created(&change, &mut self.created);
        self.changes.push(change);
        Ok(())
    }

    /// The next position of id sequence `seq`, never given before (also when this
    /// transaction is redone or fails: the position is then left unused).
    pub fn allocate(&mut self, seq: IdSeq) -> Result<u64, Error<D>> {
        Ok(self.ids.draw(seq)?)
    }

    /// The keys read so far.
    pub fn reads(&self) -> &KeySet<D::Key> {
        &self.reads
    }

    /// The keys written so far.
    pub fn writes(&self) -> &KeySet<D::Key> {
        &self.writes
    }

    /// The change set so far, in order.
    pub fn changes(&self) -> &[D::Change] {
        &self.changes
    }

    /// The point to return to.
    pub fn savepoint(&self) -> Savepoint<D> {
        Savepoint {
            work: self.work.clone(),
            writes: self.writes.clone(),
            merges: self.merges.clone(),
            created: self.created.clone(),
            changes: self.changes.len(),
        }
    }

    /// Return to `point`: the changes after it are dropped.
    pub fn rollback(&mut self, point: Savepoint<D>) {
        self.work = point.work;
        self.writes = point.writes;
        self.merges = point.merges;
        self.created = point.created;
        self.changes.truncate(point.changes);
    }

    pub(crate) fn into_parts(self) -> TxnParts<D> {
        TxnParts {
            base: self.base,
            work: self.work,
            reads: self.reads,
            predicates: self.predicates,
            writes: self.writes,
            merges: self.merges,
            created: self.created,
            changes: self.changes,
        }
    }
}

/// A prepared transaction, as the commit takes it.
pub(crate) struct TxnParts<D: DocModel> {
    pub(crate) base: Snapshot<D>,
    pub(crate) work: D,
    pub(crate) reads: KeySet<D::Key>,
    pub(crate) predicates: Vec<Arc<dyn ReadPredicate<D>>>,
    pub(crate) writes: KeySet<D::Key>,
    pub(crate) merges: KeySet<D::Key>,
    pub(crate) created: KeySet<D::Key>,
    pub(crate) changes: Vec<D::Change>,
}
