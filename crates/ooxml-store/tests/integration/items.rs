//! A positional model for the store's tests: named items in a list, changed by place, so a
//! commit's changes are moved past those published since its snapshot ([`DocModel::rebase`],
//! design section 5); and the ops on it. Shared by the integration tests and tests/loom.rs.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
#![allow(dead_code)]

use ooxml_store::{Command, DocModel, Error, First, KeySet, Txn};
use std::sync::Arc;

/// Named items in order, each with a value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Items(pub Arc<Vec<(u32, i64)>>);

impl Items {
    /// Items named 0 .. `count`, each 0.
    pub fn named(count: u32) -> Self {
        Items(Arc::new((0..count).map(|n| (n, 0)).collect()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Item `name` goes in at `at`.
    Insert { at: usize, name: u32 },
    /// The item at `at`, which must be `name`, gets `value`.
    Set { at: usize, name: u32, value: i64 },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("the item at {0} is not the one the change names")]
pub struct Stale(pub usize);

impl DocModel for Items {
    type Key = u32;
    type Change = Change;
    type Error = Stale;
    type Setup = ();
    type Delta = ();

    fn delta(_: &Self, _: &Self) {}

    fn apply(&mut self, change: &Change) -> Result<(), Stale> {
        let items = Arc::make_mut(&mut self.0);
        match *change {
            Change::Insert { at, name } if at <= items.len() => items.insert(at, (name, 0)),
            Change::Set { at, name, value } if items.get(at).is_some_and(|i| i.0 == name) => items[at].1 = value,
            Change::Insert { at, .. } | Change::Set { at, .. } => return Err(Stale(at)),
        }
        Ok(())
    }

    fn inverse(&self, change: &Change) -> Result<Change, Stale> {
        match *change {
            Change::Set { at, name, .. } => Ok(Change::Set {
                at,
                name,
                value: self.0.get(at).ok_or(Stale(at))?.1,
            }),
            Change::Insert { at, .. } => Err(Stale(at)),
        }
    }

    fn agrees(&self, other: &Self, keys: &KeySet<u32>) -> bool {
        let value = |items: &Items, name: &u32| items.0.iter().find(|i| i.0 == *name).map(|i| i.1);
        match keys.keys() {
            Some(keys) => keys.iter().all(|k| value(self, k) == value(other, k)),
            None => self == other,
        }
    }

    fn written(change: &Change, into: &mut KeySet<u32>) {
        match change {
            Change::Insert { name, .. } | Change::Set { name, .. } => into.insert(*name),
        }
    }

    fn rebase(change: &Change, past: &Change, first: First) -> Result<Change, Stale> {
        let Change::Insert { at: there, .. } = *past else {
            return Ok(change.clone());
        };
        Ok(match *change {
            Change::Set { at, name, value } if there <= at => Change::Set {
                at: at + 1,
                name,
                value,
            },
            // two inserts at one place: the one published first comes first
            Change::Insert { at, name } if there < at || (there == at && first == First::Past) => {
                Change::Insert { at: at + 1, name }
            }
            ref other => other.clone(),
        })
    }
}

/// Insert a new item before the item named `before`, or add to the value of item `name`.
#[derive(Debug, Clone)]
pub enum Op {
    Insert { name: u32, before: u32 },
    Add { name: u32, add: i64 },
}

impl Command<Items> for Op {
    type Out = ();
    type Error = Error<Items>;

    fn validate(&self, _doc: &Items) -> Result<(), Self::Error> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, Items>) -> Result<(), Self::Error> {
        let place = |tx: &Txn<'_, Items>, name: u32| tx.doc().0.iter().position(|i| i.0 == name);
        match *self {
            Op::Insert { name, before } => {
                // depends on the item it goes before, not on any other's place
                tx.read(before);
                let at = place(tx, before).unwrap_or(tx.doc().0.len());
                tx.write(Change::Insert { at, name })
            }
            Op::Add { name, add } => {
                tx.read(name);
                let Some(at) = place(tx, name) else { return Ok(()) };
                let value = tx.doc().0[at].1 + add;
                tx.write(Change::Set { at, name, value })
            }
        }
    }
}
