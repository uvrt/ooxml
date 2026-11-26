//! A commit whose keys are disjoint from those committed since its snapshot is rebased
//! past their changes (design section 5): a model whose changes are positional -- a list of
//! named items, changed by place -- gives, from several threads, what a serial replay in
//! commit order gives, and commits without preparing again when only positions moved.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common;

use crate::items::{Change, Items, Op};
use common::{still, Rng};
use ooxml_store::{Batch, Caller, Command, DocModel, First, Store, StoreConfig, Txn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

/// A command that, once prepared the first time, waits at `gate` twice: for another writer
/// to commit after the snapshot it prepared on, which forces its commit to be rebased (or,
/// where that fails, prepared again, without waiting).
struct Gated<'a, C> {
    op: C,
    gate: &'a Barrier,
    waited: AtomicBool,
}

impl<'a, C> Gated<'a, C> {
    fn new(op: C, gate: &'a Barrier) -> Self {
        Gated {
            op,
            gate,
            waited: AtomicBool::new(false),
        }
    }
}

impl<C: Command<Items>> Command<Items> for Gated<'_, C> {
    type Out = C::Out;
    type Error = C::Error;

    fn validate(&self, doc: &Items) -> Result<(), Self::Error> {
        self.op.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, Items>) -> Result<C::Out, Self::Error> {
        let out = self.op.prepare(tx)?;
        if !self.waited.swap(true, Ordering::SeqCst) {
            self.gate.wait(); // prepared
            self.gate.wait(); // the other writer committed
        }
        Ok(out)
    }
}

/// `batch` prepared on a list of 8 items, then `writer` committed, then the batch committed:
/// how many times the batch was prepared, and whether the list equals the serial replay
/// (the writer, then the batch).
fn batch_past_a_writer(batch: Vec<Op>, writer: Op) -> (u32, Items, Items) {
    let store = Store::new(Items::named(8), StoreConfig::default(), still());
    let gate = Barrier::new(2);
    let attempts = std::thread::scope(|scope| {
        let batcher = scope.spawn(|| {
            let gated = Gated::new(Batch::new(batch.clone()), &gate);
            let done = store.run(&Caller::named("batch"), &gated).unwrap();
            assert!(done.out.iter().all(Result::is_ok));
            done.attempts
        });
        gate.wait(); // the batch prepared
        store.run(&Caller::named("writer"), &writer).unwrap();
        gate.wait(); // let it commit
        batcher.join().unwrap()
    });
    let serial = Store::new(Items::named(8), StoreConfig::default(), still());
    serial.run(&Caller::anonymous(), &writer).unwrap();
    serial.run(&Caller::anonymous(), &Batch::new(batch)).unwrap();
    (
        attempts,
        store.snapshot().doc().clone(),
        serial.snapshot().doc().clone(),
    )
}

#[test]
fn a_batchs_second_change_is_moved_past_a_commit_as_it_stands_after_the_first() {
    // the batch inserts before item 2, then sets item 3 (at 4 once the first went in); the
    // writer inserted before item 4 meanwhile. Moved past the batch's first change, the
    // writer's insertion is at 5, after item 3: the second change stays at 4 (moved past the
    // writer's insertion as made, at 4, it would name the wrong item and run again)
    let batch = vec![Op::Insert { name: 50, before: 2 }, Op::Add { name: 3, add: 7 }];
    let (attempts, concurrent, serial) = batch_past_a_writer(batch, Op::Insert { name: 60, before: 4 });
    assert_eq!(attempts, 1, "rebased, not prepared again");
    assert_eq!(concurrent, serial);
    assert_eq!(concurrent.0[4], (3, 7));
}

#[test]
fn the_tie_rule_in_a_sequence_puts_the_commit_published_first_first() {
    // both insert before item 3; the writer published first, so its item comes first, and
    // the batch's set of item 3 lands after both
    let batch = vec![
        Op::Insert { name: 50, before: 3 },
        Op::Add { name: 3, add: 1 },
        Op::Insert { name: 51, before: 3 },
    ];
    let (attempts, concurrent, serial) = batch_past_a_writer(batch, Op::Insert { name: 60, before: 3 });
    assert_eq!(attempts, 1);
    assert_eq!(concurrent, serial);
    let names: Vec<u32> = concurrent.0.iter().map(|i| i.0).collect();
    assert_eq!(names, [0, 1, 2, 60, 50, 51, 3, 4, 5, 6, 7]);
}

#[test]
fn the_tie_rule_case_by_case() {
    // the model's own rule, each way round: where two insert at one place, the one
    // published first comes first; anywhere else, the place decides
    let at = |at| Change::Insert { at, name: 9 };
    let other = Change::Insert { at: 2, name: 8 };
    assert_eq!(
        Items::rebase(&at(2), &other, First::Past).unwrap(),
        at(3),
        "theirs first"
    );
    assert_eq!(
        Items::rebase(&at(2), &other, First::Change).unwrap(),
        at(2),
        "this first"
    );
    for first in [First::Past, First::Change] {
        assert_eq!(Items::rebase(&at(1), &other, first).unwrap(), at(1), "before: kept");
        assert_eq!(Items::rebase(&at(3), &other, first).unwrap(), at(4), "after: moved");
    }
    let set = Change::Set {
        at: 2,
        name: 2,
        value: 1,
    };
    for first in [First::Past, First::Change] {
        // an insert where an item is goes before it, whichever was published first
        assert_eq!(
            Items::rebase(&set, &other, first).unwrap(),
            Change::Set {
                at: 3,
                name: 2,
                value: 1
            }
        );
    }
}

#[test]
fn rebased_commits_equal_a_serial_replay() {
    for seed in [3u64, 5, 8] {
        let first: Vec<(u32, i64)> = (0..12).map(|n| (n, 0)).collect();
        let store = Store::new(Items(Arc::new(first.clone())), StoreConfig::default(), still());
        let mut records = Vec::new();
        // Writer 0's first op is prepared, then writer 1 inserts at the front and commits,
        // then writer 0 commits: its change is moved past the insertion (a rebase, every run).
        // The others start once those two committed; then all interleave as they happen to.
        let gate = Barrier::new(2);
        let start = Barrier::new(6);
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..6u64)
                .map(|t| {
                    let store = &store;
                    let (gate, start) = (&gate, &start);
                    scope.spawn(move || {
                        let mut rng = Rng(seed ^ (t + 1).wrapping_mul(0x9E37));
                        let caller = Caller::named(&format!("writer {t}"));
                        let mut forced = Vec::new();
                        if t == 0 {
                            let op = Op::Add { name: 11, add: 1 };
                            let done = store.run(&caller, &Gated::new(op.clone(), gate)).unwrap();
                            forced.push((done.version.seq(), op, done.version));
                        } else if t == 1 {
                            let op = Op::Insert { name: 999, before: 0 };
                            gate.wait(); // writer 0 prepared
                            let done = store.run(&caller, &op).unwrap();
                            gate.wait(); // let it commit
                            forced.push((done.version.seq(), op, done.version));
                        }
                        start.wait();
                        forced.extend((0..80u32).map(|k| {
                            let op = if rng.below(3) == 0 {
                                Op::Insert {
                                    name: 1000 + t as u32 * 100 + k,
                                    before: rng.below(12) as u32,
                                }
                            } else {
                                Op::Add {
                                    name: rng.below(12) as u32,
                                    add: rng.below(50) as i64,
                                }
                            };
                            let done = store.run(&caller, &op).unwrap();
                            (done.version.seq(), op, done.version)
                        }));
                        forced
                    })
                })
                .collect();
            for handle in handles {
                records.extend(handle.join().unwrap());
            }
        });
        records.sort_by_key(|r| r.0);
        let serial = Store::new(Items(Arc::new(first)), StoreConfig::default(), still());
        for (seq, op, version) in &records {
            let done = serial.run(&Caller::anonymous(), op).unwrap();
            assert_eq!(done.version.seq(), *seq);
            assert_eq!(done.version.doc(), version.doc(), "document at {seq}");
        }
        let stats = store.stats();
        assert_eq!(stats.commits, records.len() as u64);
        assert!(stats.rebased > 0, "{stats:?}");
    }
}

#[test]
fn a_moved_position_commits_without_preparing_again() {
    let store = Store::new(
        Items(Arc::new((0..4).map(|n| (n, 0)).collect())),
        StoreConfig::default(),
        still(),
    );
    let base = store.snapshot();
    // prepared on `base`, committed after an insertion before its item
    let mut tx_doc = base.doc().clone();
    tx_doc
        .apply(&Change::Set {
            at: 2,
            name: 2,
            value: 7,
        })
        .unwrap();
    store
        .run(&Caller::anonymous(), &Op::Insert { name: 50, before: 0 })
        .unwrap();
    let rebased = Items::rebase(
        &Change::Set {
            at: 2,
            name: 2,
            value: 7,
        },
        &Change::Insert { at: 0, name: 50 },
        First::Past,
    )
    .unwrap();
    assert_eq!(
        rebased,
        Change::Set {
            at: 3,
            name: 2,
            value: 7
        }
    );
    let done = store.run(&Caller::anonymous(), &Op::Add { name: 2, add: 7 }).unwrap();
    assert_eq!(done.attempts, 1);
    assert_eq!(done.version.doc().0[3], (2, 7));
}

#[test]
fn rebased_batches_beside_writers_equal_a_serial_replay() {
    // writers of one op and of batches of two to four, on random places: each batch's
    // change sequence is moved past the commits since its snapshot as one
    let mut rebased = 0;
    for seed in [13u64, 21, 34, 55] {
        let store = Store::new(Items::named(12), StoreConfig::default(), still());
        let mut records = Vec::new();
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..6u64)
                .map(|t| {
                    let store = &store;
                    scope.spawn(move || {
                        let mut rng = Rng(seed ^ (t + 1).wrapping_mul(0x5DEE_CE66));
                        let caller = Caller::named(&format!("writer {t}"));
                        (0..60u32)
                            .map(|k| {
                                let size = if t % 2 == 1 { 2 + rng.below(3) as u32 } else { 1 };
                                let ops = (0..size)
                                    .map(|j| {
                                        if rng.below(3) == 0 {
                                            Op::Insert {
                                                name: 1000 + (t as u32 * 100 + k) * 10 + j,
                                                before: rng.below(12) as u32,
                                            }
                                        } else {
                                            Op::Add {
                                                name: rng.below(12) as u32,
                                                add: rng.below(50) as i64,
                                            }
                                        }
                                    })
                                    .collect();
                                let batch = Batch::new(ops);
                                let done = store.run(&caller, &batch).unwrap();
                                (done.version.seq(), batch, done.version)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for handle in handles {
                records.extend(handle.join().unwrap());
            }
        });
        records.sort_by_key(|r| r.0);
        let serial = Store::new(Items::named(12), StoreConfig::default(), still());
        for (seq, batch, version) in &records {
            let done = serial.run(&Caller::anonymous(), batch).unwrap();
            assert_eq!(done.version.seq(), *seq, "seed {seed}");
            assert_eq!(done.version.doc(), version.doc(), "seed {seed}: document at {seq}");
        }
        rebased += store.stats().rebased;
    }
    assert!(rebased > 0);
}

#[test]
fn a_batch_outlasting_two_hundred_commits_beside_it_still_rebases() {
    // the default rebase window keeps the change sets a long batch needs (StoreConfig's
    // measured choice): two hundred writers' commits land while it prepares
    let store = Store::new(Items::named(8), StoreConfig::default(), still());
    let gate = Barrier::new(2);
    let batch = vec![Op::Insert { name: 50, before: 2 }, Op::Add { name: 3, add: 7 }];
    let attempts = std::thread::scope(|scope| {
        let batcher = scope.spawn(|| {
            let gated = Gated::new(Batch::new(batch.clone()), &gate);
            store.run(&Caller::named("batch"), &gated).unwrap().attempts
        });
        gate.wait(); // the batch prepared
        for k in 0..200 {
            store
                .run(&Caller::named("writer"), &Op::Add { name: 6, add: k })
                .unwrap();
        }
        gate.wait(); // let it commit
        batcher.join().unwrap()
    });
    assert_eq!(attempts, 1, "rebased past them all, not prepared again");
    assert!(store.snapshot().doc().0.contains(&(3, 7)));
}
