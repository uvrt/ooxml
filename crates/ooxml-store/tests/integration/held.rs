//! A transaction an owner holds open across calls (a host's batch): its commands' versions
//! read by the owner alone, numbered as the commit numbers the step; marks to return to; one
//! commit and one step at the end, or nothing when it is dropped; the gate held meanwhile,
//! so other writers publish after it and undo waits for it to end.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common;

use common::{still, Cells, Change, Put};
use ooxml_store::{Caller, Command, Store, StoreConfig, StoreError, TxnOwner};
use std::sync::Arc;
use std::time::Duration;

fn fresh() -> Store<Cells> {
    Store::new(Cells::default(), StoreConfig::default(), still())
}

const A: TxnOwner = TxnOwner::new(1);
const B: TxnOwner = TxnOwner::new(2);

/// Run `command` in `owner`'s held transaction, at a savepoint (as a batch's op).
fn op(store: &Store<Cells>, owner: TxnOwner, cells: &[(u32, i64)]) -> bool {
    let put = Put(cells.to_vec());
    store
        .in_txn(owner, |tx| {
            let point = tx.savepoint();
            if put.prepare(tx).is_err() {
                tx.rollback(point);
            }
        })
        .unwrap()
        .changed
}

#[test]
fn a_held_transaction_is_one_commit_and_one_step_read_by_its_owner_alone() {
    let store = fresh();
    let base = store.open_txn(A, &()).unwrap();
    assert!(op(&store, A, &[(1, 10)]));
    let first = store.txn_current(A).unwrap();
    assert!(op(&store, A, &[(2, 20)]));
    let second = store.txn_current(A).unwrap();
    // the owner reads each command's version, numbered as the step will be; the head is not moved
    assert_eq!((first.doc().get(1), second.doc().get(2)), (10, 20));
    assert_eq!((first.number(), second.number()), (1, 1));
    assert!(first.seq() < second.seq(), "each version its own number");
    assert!(Arc::ptr_eq(&store.snapshot(), &base));
    assert!(store.txn_current(B).is_none(), "another owner reads the head");
    let done = store.commit_txn(A, &Caller::named("batch")).unwrap();
    assert!(done.changed);
    assert_eq!(
        done.version.seq(),
        second.seq(),
        "the version the owner read last, published"
    );
    assert_eq!(done.version.number(), 1);
    assert_eq!((done.version.doc().get(1), done.version.doc().get(2)), (10, 20));
    let steps = store.steps();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].caller.as_deref(), Some("batch"));
    store.undo().unwrap();
    assert_eq!(store.snapshot().doc().get(1), 0);
    assert!(!store.txn_held());
}

#[test]
fn a_revert_returns_to_its_mark_and_a_drop_publishes_nothing() {
    let store = fresh();
    store.open_txn(A, &()).unwrap();
    op(&store, A, &[(1, 1)]);
    let mark = store.txn_mark(A).unwrap();
    let at_mark = store.txn_current(A).unwrap();
    op(&store, A, &[(1, 2), (2, 2)]);
    let (_, back) = store.txn_revert(A, mark).unwrap().unwrap();
    assert_eq!((back.doc(), back.number()), (at_mark.doc(), at_mark.number()));
    assert!(back.seq() > at_mark.seq(), "a version of its own");
    assert!(store.txn_revert(A, mark).unwrap().is_none(), "nothing since");
    let (_, start) = store.txn_revert(A, store.txn_start()).unwrap().unwrap();
    assert_eq!(start.doc(), store.snapshot().doc());
    assert!(
        store.txn_revert(A, mark).is_err(),
        "a mark after the one returned to is let go"
    );
    op(&store, A, &[(3, 3)]);
    assert!(store.drop_txn(A));
    assert!(!store.drop_txn(A));
    assert_eq!(store.snapshot().seq().0, 0, "nothing published");
    assert!(store.steps().is_empty());
    // a step after it numbered as if it never was
    let done = store.run(&Caller::anonymous(), &Put(vec![(9, 9)])).unwrap();
    assert_eq!(done.version.number(), 1);
}

#[test]
fn the_same_owner_is_refused_and_undo_waits_for_the_end() {
    let store = fresh();
    store.run(&Caller::anonymous(), &Put(vec![(1, 1)])).unwrap();
    store.open_txn(A, &()).unwrap();
    assert!(matches!(store.open_txn(A, &()), Err(StoreError::TxnHeld)));
    assert!(matches!(store.undo(), Err(StoreError::GroupOpen)));
    assert!(matches!(store.open_group(), Err(StoreError::TxnHeld)));
    assert!(matches!(store.in_txn(B, |_| ()), Err(StoreError::NotHeld)));
    let empty = store.commit_txn(A, &Caller::anonymous()).unwrap();
    assert!(!empty.changed, "nothing done: no step");
    assert_eq!(store.steps().len(), 1);
    store.undo().unwrap();
}

#[test]
fn a_panic_in_a_command_returns_the_transaction_to_before_it() {
    let store = fresh();
    store.open_txn(A, &()).unwrap();
    op(&store, A, &[(1, 1)]);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.in_txn(A, |tx| {
            tx.write(Change::Set(2, Some(2))).unwrap();
            panic!("in the command");
        })
    }));
    assert!(caught.is_err());
    let done = store.commit_txn(A, &Caller::anonymous()).unwrap();
    assert_eq!((done.version.doc().get(1), done.version.doc().get(2)), (1, 0));
}

#[test]
fn another_owner_and_other_writers_wait_for_the_held_transaction() {
    let store = Arc::new(fresh());
    store.open_txn(A, &()).unwrap();
    op(&store, A, &[(1, 1)]);
    let writer = {
        let store = Arc::clone(&store);
        std::thread::spawn(move || store.run(&Caller::named("w"), &Put(vec![(2, 2)])).unwrap().version)
    };
    let other = {
        let store = Arc::clone(&store);
        std::thread::spawn(move || {
            store.open_txn(B, &()).unwrap();
            let seen = store.txn_current(B).unwrap().doc().get(1);
            store.commit_txn(B, &Caller::anonymous()).unwrap();
            seen
        })
    };
    std::thread::sleep(Duration::from_millis(50));
    assert!(!writer.is_finished() && !other.is_finished(), "both wait");
    let done = store.commit_txn(A, &Caller::named("batch")).unwrap();
    let written = writer.join().unwrap();
    assert_eq!(other.join().unwrap(), 1, "opened after the commit");
    assert!(written.seq() > done.version.seq(), "the writer published after it");
    assert_eq!(store.snapshot().doc().get(2), 2);
}
