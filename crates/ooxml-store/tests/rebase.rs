//! A commit whose keys are disjoint from those committed since its snapshot is rebased
//! past their changes (design section 5): a model whose changes are positional -- a list of
//! named items, changed by place -- gives, from several threads, what a serial replay in
//! commit order gives, and commits without preparing again when only positions moved.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::{still, Rng};
use ooxml_store::{Caller, Command, DocModel, Error, KeySet, Store, StoreConfig, Txn};
use std::sync::{Arc, Barrier};

/// Named items in order, each with a value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Items(Arc<Vec<(u32, i64)>>);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Change {
    /// Item `name` goes in at `at`.
    Insert { at: usize, name: u32 },
    /// The item at `at`, which must be `name`, gets `value`.
    Set { at: usize, name: u32, value: i64 },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("the item at {0} is not the one the change names")]
struct Stale(usize);

impl DocModel for Items {
    type Key = u32;
    type Change = Change;
    type Error = Stale;

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

    fn rebase(change: &Change, past: &Change) -> Result<Change, Stale> {
        let Change::Insert { at: there, .. } = *past else {
            return Ok(change.clone());
        };
        Ok(match *change {
            Change::Set { at, name, value } if there <= at => Change::Set {
                at: at + 1,
                name,
                value,
            },
            Change::Insert { at, name } if there <= at => Change::Insert { at: at + 1, name },
            ref other => other.clone(),
        })
    }
}

/// Insert a new item before the item named `before`, or add to the value of item `name`.
#[derive(Debug, Clone)]
enum Op {
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

/// An op that, once prepared, waits at `gate` twice: for another writer to commit after the
/// snapshot it prepared on, which forces its commit to be rebased.
struct Gated<'a> {
    op: Op,
    gate: &'a Barrier,
}

impl Command<Items> for Gated<'_> {
    type Out = ();
    type Error = Error<Items>;

    fn validate(&self, doc: &Items) -> Result<(), Self::Error> {
        self.op.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, Items>) -> Result<(), Self::Error> {
        self.op.prepare(tx)?;
        self.gate.wait(); // prepared
        self.gate.wait(); // the other writer committed
        Ok(())
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
                            let done = store.run(&caller, &Gated { op: op.clone(), gate }).unwrap();
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
