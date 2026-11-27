//! A refused op of a transaction that goes on (design phase 3 §4: everything an op's outcome
//! depends on, applied or refused, is in the transaction's read set): a command's refusal
//! reads what it rests on (`Txn::run_op`, `Command::read_refusal`), so a commit meanwhile
//! that changes it is a conflict; a host's op that failed outside a command reads the whole
//! document (`Txn::take_back`).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common;

use common::{still, Cells, Change, Guard, Put};
use ooxml_store::{Batch, Caller, Command, KeySet, Store, StoreConfig, Txn};
use std::sync::Arc;

fn store() -> Store<Cells> {
    Store::new(
        Cells(Arc::new([(0, 1)].into_iter().collect())),
        StoreConfig::default(),
        still(),
    )
}

/// A batch whose second op refuses while cell 1 is empty.
fn batch() -> Batch<Guard> {
    Batch::new(vec![
        Guard {
            on: 0,
            write: 2,
            value: 5,
        },
        Guard {
            on: 1,
            write: 3,
            value: 7,
        },
    ])
}

#[test]
fn a_refused_op_reads_what_its_refusal_rests_on_and_a_writer_changing_it_conflicts() {
    let store = store();
    let mut tx = store.begin(&());
    let outs = tx.run_op(&batch()).unwrap();
    assert!(outs[0].is_ok() && outs[1].is_err());
    assert_eq!(tx.reads(), &KeySet::of([0, 1]), "the refused op read cell 1");
    assert_eq!(tx.len(), 1, "its change taken back");
    // a writer fills cell 1: the refusal no longer holds, so the batch conflicts
    store.run(&Caller::named("writer"), &Put(vec![(1, 10)])).unwrap();
    assert!(store.commit(&Caller::named("batch"), tx).is_none());
    // prepared again on the head, it applies both, as the serial order gives
    let done = store.run(&Caller::named("batch"), &batch()).unwrap();
    assert!(done.out.iter().all(Result::is_ok));
    assert_eq!(done.version.doc().get(3), 7);
}

#[test]
fn a_writer_beside_what_a_refusal_rests_on_does_not_conflict() {
    let store = store();
    let mut tx = store.begin(&());
    tx.run_op(&batch()).unwrap();
    store.run(&Caller::named("writer"), &Put(vec![(9, 10)])).unwrap();
    let done = store.commit(&Caller::named("batch"), tx).unwrap();
    assert_eq!((done.version.doc().get(2), done.version.doc().get(3)), (5, 0));
}

/// A command that names nothing its refusal rests on: the store's default.
struct Refuses;

impl Command<Cells> for Refuses {
    type Out = ();
    type Error = common::Fail;

    fn validate(&self, _doc: &Cells) -> Result<(), common::Fail> {
        Err(common::Fail::Refused)
    }

    fn prepare(&self, _tx: &mut Txn<'_, Cells>) -> Result<(), common::Fail> {
        Ok(())
    }
}

#[test]
fn a_refusal_that_names_nothing_reads_the_whole_document() {
    let store = store();
    let mut tx = store.begin(&());
    assert!(tx.run_op(&Refuses).is_err());
    assert_eq!(tx.reads(), &KeySet::Everything);
}

#[test]
fn a_host_op_failing_outside_a_command_reads_the_whole_document_one_refused_by_one_what_it_read() {
    let store = store();
    // failed in the op's own code: nothing says what it rests on
    let mut tx = store.begin(&());
    let point = tx.savepoint();
    tx.write(Change::Set(4, Some(1))).unwrap();
    tx.take_back(point);
    assert!(tx.is_empty());
    assert_eq!(tx.reads(), &KeySet::Everything);
    // ended in a command's refusal: that read what it rests on
    let mut tx = store.begin(&());
    let point = tx.savepoint();
    assert!(tx
        .run_op(&Guard {
            on: 1,
            write: 3,
            value: 7
        })
        .is_err());
    tx.take_back(point);
    assert_eq!(tx.reads(), &KeySet::of([1]));
    // a command that applied, then the op failing after it: the whole document again
    let mut tx = store.begin(&());
    let point = tx.savepoint();
    tx.run_op(&Guard {
        on: 0,
        write: 3,
        value: 7,
    })
    .unwrap();
    tx.take_back(point);
    assert_eq!(tx.reads(), &KeySet::Everything);
}
