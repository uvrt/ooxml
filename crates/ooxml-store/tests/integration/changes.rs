//! A version's changes ([`ooxml_store::Version::changes`]): what it changed against the
//! version a reader held before it -- a commit's against the head it was published after,
//! an undo's and a redo's likewise, a held transaction's command against the owner's
//! version before it (not the transaction's base), a revert against the version reverted
//! from -- worked out once when first asked, the earlier document let go then.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{still, Cells, Put};
use ooxml_store::{Caller, Command, Store, StoreConfig, TxnOwner};
use std::collections::BTreeMap;
use std::sync::Arc;

fn fresh() -> Store<Cells> {
    Store::new(Cells::default(), StoreConfig::default(), still())
}

fn put(store: &Store<Cells>, cells: &[(u32, i64)]) {
    store.run(&Caller::anonymous(), &Put(cells.to_vec())).unwrap();
}

fn changes(store: &Store<Cells>) -> BTreeMap<u32, (i64, i64)> {
    (*store.snapshot().changes().unwrap()).clone()
}

#[test]
fn commits_undos_and_redos_change_against_the_version_before() {
    let store = fresh();
    assert!(store.snapshot().changes().is_none(), "a first version has none");
    put(&store, &[(1, 5)]);
    put(&store, &[(1, 7), (2, 3)]);
    let head = store.snapshot();
    assert!(head.holds_before(), "not asked yet");
    assert_eq!(changes(&store), BTreeMap::from([(1, (5, 7)), (2, (0, 3))]));
    assert!(!head.holds_before(), "the earlier document let go once worked out");
    assert!(Arc::ptr_eq(&head.changes().unwrap(), &head.changes().unwrap()), "kept");
    store.undo().unwrap();
    assert_eq!(changes(&store), BTreeMap::from([(1, (7, 5)), (2, (3, 0))]));
    store.redo().unwrap();
    assert_eq!(changes(&store), BTreeMap::from([(1, (5, 7)), (2, (0, 3))]));
}

#[test]
fn a_held_transactions_versions_change_against_the_owners_version_before() {
    const A: TxnOwner = TxnOwner::new(1);
    let store = fresh();
    store.open_txn(A, &()).unwrap();
    let op = |cells: &[(u32, i64)]| {
        let put = Put(cells.to_vec());
        store.in_txn(A, |tx| put.prepare(tx).unwrap()).unwrap();
        (*store.txn_current(A).unwrap().changes().unwrap()).clone()
    };
    assert_eq!(op(&[(1, 1)]), BTreeMap::from([(1, (0, 1))]));
    let mark = store.txn_mark(A).unwrap();
    assert_eq!(op(&[(2, 2)]), BTreeMap::from([(2, (0, 2))]), "not against the base");
    let (_, back) = store.txn_revert(A, mark).unwrap().unwrap();
    assert_eq!(*back.changes().unwrap(), BTreeMap::from([(2, (2, 0))]));
    let done = store.commit_txn(A, &Caller::anonymous()).unwrap();
    assert_eq!(*done.version.changes().unwrap(), BTreeMap::from([(1, (0, 1))]));
}
