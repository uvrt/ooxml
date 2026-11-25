//! States are numbered as a tool layer numbers them (`Version::number`): a commit one more
//! than the highest number given, a plain undo or redo the number of the state it brings
//! back, within the history's depth; numbers repeat, and `Store::numbered` finds the
//! head-most state with a number; writers side by side get numbers in publish order.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common;

use common::{still, Cells, Put, Rng};
use ooxml_store::{Caller, Command, Store, StoreConfig, StoreError, TxnOwner};

fn fresh() -> Store<Cells> {
    Store::new(Cells::default(), StoreConfig::default(), still())
}

/// The numbering rule as ooxml-edit's `OwnedHistory` keeps it: the number now, the highest
/// given, the numbers undo and redo go back to.
#[derive(Default)]
struct Model {
    number: u64,
    last: u64,
    undo: Vec<u64>,
    redo: Vec<u64>,
}

impl Model {
    fn commit(&mut self) {
        self.undo.push(self.number);
        if self.undo.len() > 50 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.last += 1;
        self.number = self.last;
    }

    fn undo(&mut self) -> bool {
        let Some(number) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.number);
        self.number = number;
        true
    }

    fn redo(&mut self) -> bool {
        let Some(number) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.number);
        self.number = number;
        true
    }
}

#[test]
fn numbers_follow_the_tool_layers_rule_in_random_sequences() {
    for seed in 0..20u64 {
        let mut rng = Rng(seed);
        let store = fresh();
        let mut model = Model::default();
        let mut value = 0i64;
        assert_eq!(store.snapshot().number(), 0);
        for _ in 0..400 {
            match rng.below(6) {
                5 => {
                    // a held transaction: ops, perhaps all taken back, then committed: one
                    // step, one number, or none
                    let owner = TxnOwner::new(1);
                    let before = store.open_txn(owner, &()).unwrap().number();
                    let mut made = 0;
                    for _ in 0..rng.below(3) {
                        value += 1;
                        let put = Put(vec![(rng.below(10) as u32, value)]);
                        store.in_txn(owner, |tx| put.prepare(tx).unwrap()).unwrap();
                        made += 1;
                        assert_eq!(
                            store.txn_current(owner).unwrap().number(),
                            model.last + 1,
                            "seed {seed}"
                        );
                    }
                    if made > 0 && rng.below(3) == 0 {
                        store.txn_revert(owner, store.txn_start()).unwrap();
                        assert_eq!(store.txn_current(owner).unwrap().number(), before, "seed {seed}");
                        made = 0;
                    }
                    store.commit_txn(owner, &Caller::named("w")).unwrap();
                    if made > 0 {
                        model.commit();
                    }
                }
                0 | 1 => {
                    value += 1;
                    store
                        .run(&Caller::named("w"), &Put(vec![(rng.below(10) as u32, value)]))
                        .unwrap();
                    model.commit();
                }
                2 | 3 => match store.undo() {
                    Ok(_) => assert!(model.undo()),
                    Err(StoreError::NothingToUndo) => assert!(!model.undo()),
                    Err(other) => panic!("{other}"),
                },
                _ => match store.redo() {
                    Ok(_) => assert!(model.redo()),
                    Err(StoreError::NothingToRedo) => assert!(!model.redo()),
                    Err(other) => panic!("{other}"),
                },
            }
            assert_eq!(store.snapshot().number(), model.number, "seed {seed}");
        }
    }
}

#[test]
fn a_held_transaction_is_one_number_given_when_it_commits_having_made_a_step() {
    let store = fresh();
    let w = Caller::named("w");
    let owner = TxnOwner::new(1);
    store.run(&w, &Put(vec![(1, 1)])).unwrap(); // 1
                                                // all taken back: the number it opened on, and the next step gets 2
    store.open_txn(owner, &()).unwrap();
    let put = Put(vec![(1, 2)]);
    store.in_txn(owner, |tx| put.prepare(tx).unwrap()).unwrap();
    assert_eq!(store.txn_current(owner).unwrap().number(), 2);
    store.txn_revert(owner, store.txn_start()).unwrap();
    assert_eq!(store.txn_current(owner).unwrap().number(), 1);
    assert!(!store.commit_txn(owner, &w).unwrap().changed);
    // a revert to a mark keeps the transaction's number
    store.open_txn(owner, &()).unwrap();
    let (first, second) = (Put(vec![(2, 1)]), Put(vec![(2, 2)]));
    store.in_txn(owner, |tx| first.prepare(tx).unwrap()).unwrap();
    let mark = store.txn_mark(owner).unwrap();
    store.in_txn(owner, |tx| second.prepare(tx).unwrap()).unwrap();
    store.txn_revert(owner, mark).unwrap();
    assert_eq!(store.txn_current(owner).unwrap().number(), 2);
    assert!(store.commit_txn(owner, &w).unwrap().changed);
    assert_eq!(store.snapshot().number(), 2);
    store.run(&w, &Put(vec![(1, 3)])).unwrap();
    assert_eq!(store.snapshot().number(), 3);
    store.undo().unwrap();
    store.undo().unwrap();
    assert_eq!(store.snapshot().number(), 1);
}

#[test]
fn a_no_op_takes_no_number() {
    let store = fresh();
    store.run(&Caller::named("w"), &Put(vec![(1, 5)])).unwrap();
    assert!(!store.run(&Caller::named("w"), &Put(vec![])).unwrap().changed);
    assert_eq!(store.snapshot().number(), 1);
}

#[test]
fn numbered_finds_the_head_most_state_when_numbers_repeat() {
    let store = fresh();
    let w = Caller::named("w");
    store.run(&w, &Put(vec![(1, 1)])).unwrap(); // 1
    store.run(&w, &Put(vec![(1, 2)])).unwrap(); // 2
    store.undo().unwrap(); // back to 1: a new version numbered 1
    let head = store.snapshot();
    assert_eq!(head.number(), 1);
    // The head is the head-most state numbered 1 (the first one, under it, is older).
    assert!(std::sync::Arc::ptr_eq(&store.numbered(1).unwrap(), &head));
    // 2 is redo's; 0 the state opened.
    assert_eq!(store.numbered(2).unwrap().doc().0.get(&1), Some(&2));
    assert_eq!(store.numbered(0).unwrap().doc().0.get(&1), None);
    store.run(&w, &Put(vec![(1, 3)])).unwrap(); // 3: redo dropped
    assert_eq!(store.snapshot().number(), 3);
    assert!(store.numbered(2).is_none());
    // Undo from 3 brings back 1: the head again, numbered as before.
    store.undo().unwrap();
    assert_eq!(store.snapshot().number(), 1);
    assert_eq!(store.snapshot().doc().0.get(&1), Some(&1));
}

#[test]
fn undo_past_the_depth_finds_nothing_and_keeps_numbers() {
    let store = fresh();
    let w = Caller::named("w");
    for value in 1..=60 {
        store.run(&w, &Put(vec![(1, value)])).unwrap();
    }
    let mut undone = 0;
    while store.undo().is_ok() {
        undone += 1;
    }
    assert_eq!(undone, 50);
    assert_eq!(store.snapshot().number(), 10);
    store.redo().unwrap();
    assert_eq!(store.snapshot().number(), 11);
}

#[test]
fn writers_side_by_side_are_numbered_in_publish_order() {
    let store = fresh();
    std::thread::scope(|scope| {
        for writer in 0..8u32 {
            let store = &store;
            scope.spawn(move || {
                let caller = Caller::named(&format!("w{writer}"));
                for k in 0..50i64 {
                    let done = store
                        .run(&caller, &Put(vec![(writer, k + 1), (100 + writer, k + 1)]))
                        .unwrap();
                    // Only commits here: a version's number is its place in publish
                    // order, which its seq is too.
                    assert!(done.changed);
                    assert_eq!(done.version.number(), done.version.seq().0);
                }
            });
        }
    });
    // Every commit took one number, none twice.
    assert_eq!(store.snapshot().number(), 8 * 50);
}

/// A store opened again goes on with the numbers given before; never once it numbered.
#[test]
fn a_reopened_store_goes_on_with_the_numbers_given() {
    let store = fresh();
    assert!(matches!(
        store.continue_numbers(5, 4),
        Err(ooxml_store::CountersError::StateAboveHighest { state: 5, highest: 4 })
    ));
    store.continue_numbers(3, 4).unwrap();
    assert_eq!((store.snapshot().number(), store.highest()), (3, 4));
    let w = Caller::named("w");
    store.run(&w, &Put(vec![(1, 1)])).unwrap();
    assert_eq!(store.snapshot().number(), 5);
    let listed = store.listed();
    assert_eq!(
        listed
            .steps
            .iter()
            .map(|s| (s.from_number, s.number))
            .collect::<Vec<_>>(),
        vec![(3, 5)]
    );
    store.undo().unwrap();
    assert_eq!(store.snapshot().number(), 3);
    assert_eq!(listed.highest, 5);
    assert!(matches!(
        store.continue_numbers(0, 9),
        Err(ooxml_store::CountersError::AlreadyNumbered)
    ));
}
