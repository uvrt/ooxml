//! loom explores every interleaving of the commit path: two writers whose reads and writes
//! cross, a reader beside them, and an undo racing a commit; commits whose head moves between
//! their replay (outside the commit lock) and their publish; a command behind the gate beside
//! a writer and a reader, and beside an undo. Every run equals a serial run in commit order. Run with
//! `RUSTFLAGS="--cfg loom" cargo test -p ooxml-store --test loom --release`.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
#![cfg(loom)]

#[path = "integration/common.rs"]
mod common;

use common::{still, Cells, Fail, Mix, Put};
use ooxml_store::{Caller, Seq, Store, StoreConfig};
use ooxml_store::{Command, Txn};
use std::sync::Arc;

fn serial(order: &[&Mix]) -> Cells {
    let store = Store::new(Cells::default(), StoreConfig::default(), still());
    for command in order {
        store.run(&Caller::anonymous(), *command).unwrap();
    }
    store.snapshot().doc().clone()
}

#[test]
fn crossing_writers_and_a_reader_equal_a_serial_order() {
    loom::model(|| {
        let store = Arc::new(Store::new(Cells::default(), StoreConfig::default(), still()));
        let a = Mix {
            reads: vec![1],
            write: 2,
            add: 5,
            dawdle: false,
        };
        let b = Mix {
            reads: vec![2],
            write: 1,
            add: 7,
            dawdle: false,
        };
        let spawn = |command: Mix| {
            let store = Arc::clone(&store);
            loom::thread::spawn(move || store.run(&Caller::anonymous(), &command).unwrap().version.seq())
        };
        let ta = spawn(a.clone());
        let tb = spawn(b.clone());
        let seen = store.snapshot();
        let (sa, sb) = (ta.join().unwrap(), tb.join().unwrap());
        let mut seqs = [sa, sb];
        seqs.sort();
        assert_eq!(seqs, [Seq(1), Seq(2)]);
        let expected = if sa < sb { serial(&[&a, &b]) } else { serial(&[&b, &a]) };
        assert_eq!(store.snapshot().doc(), &expected);
        // What the reader saw is one of the versions of that order.
        let prefix = match (seen.seq(), sa < sb) {
            (Seq(0), _) => Cells::default(),
            (Seq(1), true) => serial(&[&a]),
            (Seq(1), false) => serial(&[&b]),
            _ => expected,
        };
        assert_eq!(seen.doc(), &prefix);
    });
}

#[test]
fn an_undo_racing_a_commit_equals_a_serial_order() {
    loom::model(|| {
        let store = Arc::new(Store::new(Cells::default(), StoreConfig::default(), still()));
        store.run(&Caller::anonymous(), &Put(vec![(1, 10)])).unwrap();
        let undoer = {
            let store = Arc::clone(&store);
            loom::thread::spawn(move || store.undo().unwrap().version.seq())
        };
        let writer = {
            let store = Arc::clone(&store);
            loom::thread::spawn(move || {
                store
                    .run(&Caller::anonymous(), &Put(vec![(2, 20)]))
                    .unwrap()
                    .version
                    .seq()
            })
        };
        let (undo, write) = (undoer.join().unwrap(), writer.join().unwrap());
        let doc = store.snapshot();
        assert_eq!(doc.seq(), Seq(3));
        // Undo takes back the newest step when it runs: the first commit, or the racing one.
        let cells = (doc.doc().get(1), doc.doc().get(2));
        if undo < write {
            assert_eq!(cells, (0, 20));
        } else {
            assert_eq!(cells, (10, 0));
        }
        assert_eq!(store.steps().len(), 1);
    });
}

/// Read every cell, then set `write` to their sum plus one: a command that read the whole
/// document, run behind the gate at once (`optimistic_tries` 0).
struct Whole {
    write: u32,
}

impl Command<Cells> for Whole {
    type Out = i64;
    type Error = Fail;

    fn validate(&self, _doc: &Cells) -> Result<(), Fail> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, Cells>) -> Result<i64, Fail> {
        tx.read_everything();
        let sum: i64 = tx.doc().0.values().sum();
        tx.write(common::Change::Set(self.write, Some(sum + 1)))?;
        Ok(sum)
    }
}

fn gated() -> StoreConfig {
    StoreConfig {
        optimistic_tries: 0,
        ..StoreConfig::default()
    }
}

#[test]
fn commits_whose_head_moves_before_they_publish_equal_a_serial_order() {
    loom::model(|| {
        let store = Arc::new(Store::new(Cells::default(), StoreConfig::default(), still()));
        let spawn = |cell: u32, value: i64| {
            let store = Arc::clone(&store);
            loom::thread::spawn(move || {
                store
                    .run(&Caller::anonymous(), &Put(vec![(cell, value)]))
                    .unwrap()
                    .version
                    .seq()
            })
        };
        // both prepared on version 0: the later one's head moves, before its publish or before
        // its check, and it is rebased past the other's either way
        let threads = [spawn(1, 10), spawn(2, 20)];
        let mut seqs: Vec<Seq> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        seqs.sort();
        assert_eq!(seqs, [Seq(1), Seq(2)]);
        let doc = store.snapshot();
        assert_eq!((doc.doc().get(1), doc.doc().get(2)), (10, 20));
        assert_eq!(doc.seq(), Seq(2));
        assert_eq!(store.steps().len(), 2);
    });
}

#[test]
fn a_command_behind_the_gate_and_a_writer_equal_a_serial_order() {
    loom::model(|| {
        let store = Arc::new(Store::new(Cells::default(), gated(), still()));
        let whole = {
            let store = Arc::clone(&store);
            loom::thread::spawn(move || {
                let done = store.run(&Caller::anonymous(), &Whole { write: 9 }).unwrap();
                assert!(done.exclusive);
                (done.version.seq(), done.out)
            })
        };
        let writer = {
            let store = Arc::clone(&store);
            loom::thread::spawn(move || {
                store
                    .run(&Caller::anonymous(), &Put(vec![(1, 10)]))
                    .unwrap()
                    .version
                    .seq()
            })
        };
        // a reader never waits for the gate
        let seen = store.snapshot();
        let ((gated_at, summed), written_at) = (whole.join().unwrap(), writer.join().unwrap());
        let doc = store.snapshot();
        // the writer's commit came wholly before the gated command's snapshot or after its
        // commit: what it summed is what the serial order gives
        if written_at < gated_at {
            assert_eq!((summed, doc.doc().get(9)), (10, 11));
        } else {
            assert_eq!((summed, doc.doc().get(9)), (0, 1));
        }
        assert_eq!(doc.doc().get(1), 10);
        assert!(seen.seq() <= doc.seq());
        assert_eq!(store.steps().len(), 2);
    });
}

#[test]
fn an_undo_racing_a_command_behind_the_gate_equals_a_serial_order() {
    loom::model(|| {
        let store = Arc::new(Store::new(Cells::default(), gated(), still()));
        store.run(&Caller::anonymous(), &Put(vec![(1, 10)])).unwrap();
        let undoer = {
            let store = Arc::clone(&store);
            loom::thread::spawn(move || store.undo().unwrap().version.seq())
        };
        let whole = {
            let store = Arc::clone(&store);
            loom::thread::spawn(move || {
                store
                    .run(&Caller::anonymous(), &Whole { write: 9 })
                    .unwrap()
                    .version
                    .seq()
            })
        };
        let (undo, gated_at) = (undoer.join().unwrap(), whole.join().unwrap());
        let doc = store.snapshot();
        assert_eq!(doc.seq(), Seq(3));
        // undo takes back the newest step when it runs: the first commit, or the gated one
        let cells = (doc.doc().get(1), doc.doc().get(9));
        if undo < gated_at {
            assert_eq!(cells, (0, 1));
        } else {
            assert_eq!(cells, (10, 0));
        }
        assert_eq!(store.steps().len(), 1);
    });
}
