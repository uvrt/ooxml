//! Step groups: the commits made while a group is open are one undo step once it closes; a
//! revert takes back the ones after a mark, with no redo of them; plain redo is as it was
//! when the group closes having made nothing.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common;

use common::{still, Cells, Put};
use ooxml_store::{Caller, Seq, Store, StoreConfig, StoreError};

fn fresh() -> Store<Cells> {
    Store::new(Cells::default(), StoreConfig::default(), still())
}

fn put(store: &Store<Cells>, cells: &[(u32, i64)]) -> Seq {
    store
        .run(&Caller::named("w"), &Put(cells.to_vec()))
        .unwrap()
        .version
        .seq()
}

fn cells(store: &Store<Cells>, which: &[u32]) -> Vec<i64> {
    let head = store.snapshot();
    which.iter().map(|cell| head.doc().get(*cell)).collect()
}

#[test]
fn a_closed_group_is_one_step_that_one_undo_takes_back() {
    let store = fresh();
    put(&store, &[(1, 1)]);
    store.open_group().unwrap();
    put(&store, &[(2, 2)]);
    put(&store, &[(3, 3)]);
    let last = put(&store, &[(2, 4)]);
    assert_eq!(store.close_group(&Caller::named("batch")).unwrap(), Some(last));
    let steps = store.steps();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[1].step, last);
    assert_eq!(steps[1].caller.as_deref(), Some("batch"));
    assert_eq!(cells(&store, &[1, 2, 3]), [1, 4, 3]);
    store.undo().unwrap();
    assert_eq!(cells(&store, &[1, 2, 3]), [1, 0, 0]);
    store.redo().unwrap();
    assert_eq!(cells(&store, &[1, 2, 3]), [1, 4, 3]);
}

#[test]
fn a_reverted_group_leaves_undo_and_redo_as_they_were() {
    let store = fresh();
    put(&store, &[(1, 1)]);
    put(&store, &[(1, 2)]);
    store.undo().unwrap();
    let redo_before = store.redoable();
    let steps_before = store.steps();
    let start = store.open_group().unwrap();
    put(&store, &[(1, 7)]);
    put(&store, &[(2, 7)]);
    let reverted = store.revert_group(start).unwrap().expect("a version");
    assert_eq!(reverted.seq(), store.snapshot().seq());
    assert_eq!(cells(&store, &[1, 2]), [1, 0]);
    assert_eq!(store.close_group(&Caller::named("batch")).unwrap(), None);
    assert_eq!(store.steps(), steps_before);
    assert_eq!(store.redoable(), redo_before);
    store.redo().unwrap();
    assert_eq!(cells(&store, &[1, 2]), [2, 0]);
}

#[test]
fn a_revert_to_a_mark_keeps_the_steps_before_it() {
    let store = fresh();
    store.open_group().unwrap();
    put(&store, &[(1, 1)]);
    let mark = store.group_mark().unwrap();
    put(&store, &[(2, 2)]);
    put(&store, &[(3, 3)]);
    store.revert_group(mark).unwrap();
    assert_eq!(store.revert_group(mark).unwrap().map(|v| v.seq()), None);
    put(&store, &[(4, 4)]);
    store.close_group(&Caller::named("batch")).unwrap();
    assert_eq!(cells(&store, &[1, 2, 3, 4]), [1, 0, 0, 4]);
    assert_eq!(store.steps().len(), 1);
    store.undo().unwrap();
    assert_eq!(cells(&store, &[1, 2, 3, 4]), [0, 0, 0, 0]);
}

#[test]
fn undo_waits_for_the_group_and_a_group_within_joins_it() {
    let store = fresh();
    put(&store, &[(1, 1)]);
    store.open_group().unwrap();
    put(&store, &[(2, 2)]);
    // a call's group within the batch's: part of the batch's step
    store.open_group().unwrap();
    put(&store, &[(3, 3)]);
    put(&store, &[(4, 4)]);
    assert!(matches!(store.undo(), Err(StoreError::GroupOpen)));
    assert_eq!(store.close_group(&Caller::named("call")).unwrap(), None);
    // a joined group's revert takes back only what came after it joined
    let again = store.open_group().unwrap();
    put(&store, &[(5, 5)]);
    store.revert_group(again).unwrap();
    store.close_group(&Caller::named("call")).unwrap();
    let last = store.close_group(&Caller::named("batch")).unwrap();
    assert_eq!(last, Some(store.snapshot().seq()));
    assert_eq!(cells(&store, &[1, 2, 3, 4, 5]), [1, 2, 3, 4, 0]);
    assert!(matches!(
        store.close_group(&Caller::anonymous()),
        Err(StoreError::NoGroup)
    ));
    assert_eq!(store.steps().len(), 2);
    store.undo().unwrap();
    assert_eq!(cells(&store, &[1, 2, 3, 4, 5]), [1, 0, 0, 0, 0]);
}

#[test]
fn a_group_longer_than_the_depth_is_one_step() {
    let store = Store::new(
        Cells::default(),
        StoreConfig {
            history_depth: 3,
            ..StoreConfig::default()
        },
        still(),
    );
    for k in 0..3 {
        put(&store, &[(k, 1)]);
    }
    store.open_group().unwrap();
    for k in 0..10 {
        put(&store, &[(10 + k, 2)]);
    }
    store.close_group(&Caller::named("batch")).unwrap();
    assert_eq!(store.steps().len(), 3);
    store.undo().unwrap();
    assert_eq!(cells(&store, &[0, 1, 2, 10, 19]), [1, 1, 1, 0, 0]);
}
