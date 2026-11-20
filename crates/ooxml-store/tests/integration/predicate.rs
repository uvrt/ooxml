//! A read by condition (design section 5): a find-like command -- every cell holding a value
//! gets another -- that reads the cells it matched by key and every other cell by condition
//! commits past a commit since its snapshot that wrote cells not holding the value, and is
//! prepared again when one written since holds it on the head or one it matched was written.
//! What it commits equals a serial run in commit order. The history keeps such a read as a
//! read of every cell, or of the cells the condition names where it names them.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common;

use common::{still, Cells, Change, Fail, Put};
use ooxml_store::{Caller, Command, KeySet, ReadPredicate, Store, StoreConfig, Txn};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The cells holding `value`.
#[derive(Debug)]
struct Holds(i64);

impl ReadPredicate<Cells> for Holds {
    fn depends(&self, head: &Cells, key: &u32) -> bool {
        head.0.get(key) == Some(&self.0)
    }
}

/// Every cell holding `find` set to `with`; before its first commit, `meanwhile` is
/// committed by another caller (once).
struct Replace<'s> {
    find: i64,
    with: i64,
    store: &'s Store<Cells>,
    meanwhile: Put,
    done: AtomicBool,
}

impl Command<Cells> for Replace<'_> {
    type Out = usize;
    type Error = Fail;

    fn validate(&self, _doc: &Cells) -> Result<(), Fail> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, Cells>) -> Result<usize, Fail> {
        tx.read_where(Arc::new(Holds(self.find)));
        let matched: Vec<u32> = tx
            .doc()
            .0
            .iter()
            .filter(|(_, v)| **v == self.find)
            .map(|(k, _)| *k)
            .collect();
        for &cell in &matched {
            tx.read(cell);
            tx.write(Change::Set(cell, Some(self.with)))?;
        }
        if !self.done.swap(true, Ordering::SeqCst) {
            self.store.run(&Caller::named("other"), &self.meanwhile)?;
        }
        Ok(matched.len())
    }
}

fn store() -> Store<Cells> {
    let cells: BTreeMap<u32, i64> = (0..10).map(|k| (k, i64::from(k % 3))).collect();
    Store::new(Cells(Arc::new(cells)), StoreConfig::default(), still())
}

fn replace<'s>(store: &'s Store<Cells>, meanwhile: Vec<(u32, i64)>) -> Replace<'s> {
    Replace {
        find: 1,
        with: 7,
        store,
        meanwhile: Put(meanwhile),
        done: AtomicBool::new(false),
    }
}

/// What the cells hold after `first`, then every cell holding 1 set to 7.
fn serial(first: &[(u32, i64)]) -> Cells {
    let mut cells: BTreeMap<u32, i64> = (0..10).map(|k| (k, i64::from(k % 3))).collect();
    for &(cell, value) in first {
        cells.insert(cell, value);
    }
    for value in cells.values_mut() {
        if *value == 1 {
            *value = 7;
        }
    }
    Cells(Arc::new(cells))
}

#[test]
fn a_commit_of_cells_not_holding_the_value_is_no_conflict() {
    let store = store();
    let meanwhile = vec![(0, 5), (2, 2)];
    let done = store
        .run(&Caller::named("finder"), &replace(&store, meanwhile.clone()))
        .unwrap();
    assert_eq!(done.attempts, 1);
    assert_eq!(done.out, 3);
    assert_eq!(store.stats().conflicts, 0);
    assert_eq!(*store.snapshot().doc(), serial(&meanwhile));
}

#[test]
fn a_cell_written_to_hold_the_value_is_a_conflict() {
    let store = store();
    let meanwhile = vec![(0, 1)];
    let done = store
        .run(&Caller::named("finder"), &replace(&store, meanwhile.clone()))
        .unwrap();
    assert_eq!(done.attempts, 2);
    assert_eq!(done.out, 4);
    assert_eq!(store.stats().conflicts, 1);
    assert_eq!(*store.snapshot().doc(), serial(&meanwhile));
}

#[test]
fn a_matched_cell_written_since_is_a_conflict() {
    let store = store();
    let meanwhile = vec![(1, 4)];
    let done = store
        .run(&Caller::named("finder"), &replace(&store, meanwhile.clone()))
        .unwrap();
    assert_eq!(done.attempts, 2);
    assert_eq!(done.out, 2);
    assert_eq!(*store.snapshot().doc(), serial(&meanwhile));
}

#[test]
fn the_history_keeps_a_read_by_condition_as_a_read_of_everything() {
    let store = store();
    store
        .run(&Caller::named("finder"), &replace(&store, vec![(0, 5)]))
        .unwrap();
    let steps = store.steps();
    let finder = steps.iter().find(|s| s.caller.as_deref() == Some("finder")).unwrap();
    assert_eq!(finder.reads, KeySet::Everything);
}

/// A condition over two cells only, which says so.
#[derive(Debug)]
struct Within(i64);

impl ReadPredicate<Cells> for Within {
    fn depends(&self, head: &Cells, key: &u32) -> bool {
        (3..5).contains(key) && head.0.get(key) == Some(&self.0)
    }

    fn history_keys(&self) -> Option<Vec<u32>> {
        Some(vec![3, 4])
    }
}

/// Cell 9 set to `.0`, having read cells 3 and 4 by [`Within`].
struct SetNine(i64);

impl Command<Cells> for SetNine {
    type Out = ();
    type Error = Fail;

    fn validate(&self, _doc: &Cells) -> Result<(), Fail> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, Cells>) -> Result<(), Fail> {
        tx.read_where(Arc::new(Within(1)));
        tx.write(Change::Set(9, Some(self.0)))?;
        Ok(())
    }
}

#[test]
fn the_history_keeps_a_read_by_condition_that_names_its_keys_as_a_read_of_those() {
    let store = store();
    store.run(&Caller::named("setter"), &SetNine(5)).unwrap();
    let steps = store.steps();
    let setter = steps.iter().find(|s| s.caller.as_deref() == Some("setter")).unwrap();
    assert_eq!(setter.reads, KeySet::of([3, 4]));
}
