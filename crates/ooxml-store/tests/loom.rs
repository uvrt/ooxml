//! loom explores every interleaving of the commit path: two writers whose reads and writes
//! cross, a reader beside them, and an undo racing a commit. Every run equals a serial run
//! in commit order. Run with
//! `RUSTFLAGS="--cfg loom" cargo test -p ooxml-store --test loom --release`.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
#![cfg(loom)]

mod common;

use common::{still, Cells, Mix, Put};
use ooxml_store::{Caller, Seq, Store, StoreConfig};
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
