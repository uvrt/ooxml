//! Undo and redo are inverses; scoped undo takes out one step or refuses an entangled one,
//! changing nothing; batches are one step; no-ops make no version; numbers and ids go on
//! after a reopen; a moved head with disjoint keys takes the changes without a redo.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common;

use common::{still, Cells, Change, Fail, Map, Mix, Put, Refused, Rng, BAD};
use ooxml_store::{Batch, Caller, Command, Counters, IdSeq, KeySet, Seq, Store, StoreConfig, StoreError, Txn};
use std::sync::Arc;

fn fresh() -> Store<Cells> {
    Store::new(Cells::default(), StoreConfig::default(), still())
}

fn put(store: &Store<Cells>, who: &str, cells: &[(u32, i64)]) -> Seq {
    store
        .run(&Caller::named(who), &Put(cells.to_vec()))
        .unwrap()
        .version
        .seq()
}

#[test]
fn undo_and_redo_are_inverses_in_random_sequences() {
    for seed in 0..20u64 {
        let mut rng = Rng(seed);
        let store = fresh();
        // The oracle: the linear history's states and where we are in it.
        let mut states = vec![Cells::default()];
        let mut at = 0usize;
        let mut last = Seq(0);
        // How many steps undo can reach (the depth is 50).
        let mut reach = 0usize;
        for _ in 0..300 {
            match rng.below(5) {
                0 | 1 => {
                    let cells: Vec<(u32, i64)> = (0..1 + rng.below(3))
                        .map(|_| (rng.below(10) as u32, rng.below(100) as i64))
                        .collect();
                    put(&store, "w", &cells);
                    states.truncate(at + 1);
                    states.push(store.snapshot().doc().clone());
                    at += 1;
                    reach = (reach + 1).min(50);
                }
                2 | 3 => match store.undo() {
                    Ok(_) => {
                        at -= 1;
                        reach -= 1;
                    }
                    Err(StoreError::NothingToUndo) => assert_eq!(reach, 0),
                    Err(other) => panic!("{other}"),
                },
                _ => match store.redo() {
                    Ok(_) => {
                        at += 1;
                        reach = (reach + 1).min(50);
                    }
                    Err(StoreError::NothingToRedo) => assert_eq!(at + 1, states.len()),
                    Err(other) => panic!("{other}"),
                },
            }
            let head = store.snapshot();
            assert_eq!(head.doc(), &states[at], "seed {seed}");
            assert!(head.seq() >= last, "numbers only go up");
            last = head.seq();
        }
    }
}

#[test]
fn undo_makes_a_new_number_with_an_earlier_content() {
    let store = fresh();
    let one = put(&store, "a", &[(1, 10)]);
    let two = put(&store, "a", &[(1, 20)]);
    let undone = store.undo().unwrap();
    assert_eq!(undone.step, two);
    assert_eq!(undone.version.seq(), Seq(3));
    assert_eq!(undone.version.doc().get(1), 10);
    assert_eq!(undone.version.last_written(&1), Seq(3));
    let redone = store.redo().unwrap();
    assert_eq!(
        (redone.step, redone.version.seq(), redone.version.doc().get(1)),
        (two, Seq(4), 20)
    );
    assert_eq!(
        store.steps().iter().map(|s| s.step).collect::<Vec<_>>(),
        vec![one, Seq(4)]
    );
}

#[test]
fn scoped_undo_takes_out_one_step_and_keeps_the_later_ones() {
    let store = fresh();
    put(&store, "a", &[(1, 1)]);
    let b = put(&store, "b", &[(2, 2)]);
    put(&store, "a", &[(3, 3)]);
    let before_plain = store.latest_step_by("b").unwrap();
    assert_eq!(before_plain, b);
    let out = store.undo_step(b, &KeySet::new()).unwrap();
    assert_eq!(out.writes, KeySet::of([2]));
    let doc = store.snapshot();
    assert_eq!((doc.doc().get(1), doc.doc().get(2), doc.doc().get(3)), (1, 0, 3));
    // Plain undo still undoes the newest remaining step.
    store.undo().unwrap();
    assert_eq!(store.snapshot().doc().get(3), 0);
    assert_eq!(store.snapshot().doc().get(1), 1);
    // The scoped undo's step comes back by its number.
    let redone = store.redo_step(b).unwrap();
    assert_eq!((redone.step, store.snapshot().doc().get(2)), (b, 2));
    assert_eq!(store.steps().last().unwrap().step, redone.version.seq());
}

#[test]
fn scoped_undo_refuses_entangled_steps_and_changes_nothing() {
    let store = fresh();
    let caller = Caller::named("a");
    let first = put(&store, "a", &[(1, 5)]);
    // A later step that read cell 1.
    store
        .run(
            &caller,
            &Mix {
                reads: vec![1],
                write: 2,
                add: 0,
                dawdle: false,
            },
        )
        .unwrap();
    let head = store.snapshot();
    match store.undo_step(first, &KeySet::new()) {
        Err(StoreError::Entangled { step, overlap }) => assert_eq!((step, overlap), (first, KeySet::of([1]))),
        other => panic!("{other:?}"),
    }
    assert!(Arc::ptr_eq(&head, &store.snapshot()), "a refusal changes nothing");
    // A later step that wrote it.
    let s = fresh();
    let first = put(&s, "a", &[(1, 5), (4, 4)]);
    put(&s, "b", &[(1, 6)]);
    assert!(matches!(
        s.undo_step(first, &KeySet::new()),
        Err(StoreError::Entangled { .. })
    ));
    // A step that wrote a shared key.
    let s = fresh();
    let only = put(&s, "a", &[(7, 1)]);
    let refused = s.undo_step(only, &KeySet::of([7]));
    assert!(matches!(refused, Err(StoreError::Entangled { overlap, .. }) if overlap == KeySet::of([7])));
    assert!(matches!(
        s.undo_step(Seq(99), &KeySet::new()),
        Err(StoreError::UnknownStep { step: Seq(99) })
    ));
}

#[test]
fn undoing_a_later_step_frees_the_earlier_one() {
    let store = fresh();
    let first = put(&store, "a", &[(5, 1)]);
    put(&store, "b", &[(5, 2)]);
    assert!(store.undo_step(first, &KeySet::new()).is_err());
    store.undo().unwrap();
    store.undo_step(first, &KeySet::new()).unwrap();
    assert_eq!(store.snapshot().doc().get(5), 0);
}

#[test]
fn redo_of_a_scoped_undo_is_refused_when_its_keys_changed_since() {
    let store = fresh();
    let a = put(&store, "a", &[(1, 1)]);
    put(&store, "b", &[(2, 2)]);
    store.undo_step(a, &KeySet::new()).unwrap();
    put(&store, "c", &[(1, 9)]);
    assert!(matches!(store.redo_step(a), Err(StoreError::Entangled { .. })));
    assert_eq!(store.snapshot().doc().get(1), 9);
    // A later step that changed it and was undone again is no longer in the way.
    store.undo().unwrap();
    store.redo_step(a).unwrap();
    assert_eq!(store.snapshot().doc().get(1), 1);
}

/// The oracle for scoped and plain undo and redo: the remaining steps' changes applied in
/// the order they were (re)applied.
#[test]
fn random_scoped_and_plain_undo_and_redo_equal_the_remaining_steps() {
    type Steps = Vec<(Seq, Vec<(u32, i64)>)>;
    for seed in 0..40u64 {
        let mut rng = Rng(seed ^ 0xABCD);
        let config = StoreConfig {
            history_depth: 1000,
            ..StoreConfig::default()
        };
        let store = Store::new(Cells::default(), config, still());
        let (mut active, mut undone, mut plain): (Steps, Steps, Steps) = (vec![], vec![], vec![]);
        for _ in 0..250 {
            let head = store.snapshot();
            let refused = |result: Result<_, StoreError<u32, Refused>>| match result {
                Err(StoreError::Entangled { .. }) => assert!(Arc::ptr_eq(&head, &store.snapshot())),
                Err(other) => panic!("{other}"),
                Ok(()) => {}
            };
            match rng.below(6) {
                0 | 1 => {
                    let cells: Vec<(u32, i64)> = (0..1 + rng.below(2))
                        .map(|_| (rng.below(12) as u32, 1 + rng.below(99) as i64))
                        .collect();
                    active.push((put(&store, "w", &cells), cells));
                    plain.clear();
                }
                2 if !active.is_empty() => {
                    let k = rng.below(active.len() as u64) as usize;
                    refused(
                        store
                            .undo_step(active[k].0, &KeySet::new())
                            .map(|_| undone.push(active.remove(k))),
                    );
                }
                3 => {
                    let k = rng.below(1 + undone.len() as u64 + plain.len() as u64) as usize;
                    let (list, k) = if k < undone.len() {
                        (&mut undone, k)
                    } else {
                        (&mut plain, k - undone.len())
                    };
                    if let Some((seq, _)) = list.get(k) {
                        // Brought back as a new step, named by the version it made.
                        refused(store.redo_step(*seq).map(|out| {
                            let (_, cells) = list.remove(k);
                            active.push((out.version.seq(), cells));
                        }));
                    }
                }
                4 => match store.undo() {
                    Ok(out) => {
                        let last = active.pop().unwrap();
                        assert_eq!(last.0, out.step);
                        plain.push(last);
                    }
                    Err(StoreError::NothingToUndo) => assert!(active.is_empty()),
                    Err(other) => panic!("{other}"),
                },
                _ => match store.redo() {
                    Ok(out) => {
                        let (_, cells) = plain.pop().unwrap();
                        active.push((out.version.seq(), cells));
                    }
                    Err(StoreError::NothingToRedo) => assert!(plain.is_empty()),
                    Err(StoreError::Entangled { .. }) => assert!(Arc::ptr_eq(&head, &store.snapshot())),
                    Err(other) => panic!("{other}"),
                },
            }
            let mut expected = Cells::default();
            for (_, cells) in &active {
                for &(cell, value) in cells {
                    Arc::make_mut(&mut expected.0).insert(cell, value);
                }
            }
            assert_eq!(store.snapshot().doc(), &expected, "seed {seed}");
            assert_eq!(
                store.steps().iter().map(|s| s.step).collect::<Vec<_>>(),
                active.iter().map(|a| a.0).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn a_batch_is_one_step_and_skips_or_refuses_a_failing_op() {
    let store = fresh();
    let caller = Caller::anonymous();
    let ops = vec![Put(vec![(1, 1)]), Put(vec![(2, 2), (BAD, 0)]), Put(vec![(3, 3)])];
    let done = store.run(&caller, &Batch::new(ops.clone())).unwrap();
    assert!(done.out[0].is_ok() && done.out[2].is_ok());
    assert!(matches!(done.out[1], Err(Fail::Store(StoreError::Model(Refused(BAD))))));
    let doc = done.version.doc();
    assert_eq!(
        (doc.get(1), doc.get(2), doc.get(3)),
        (1, 0, 3),
        "the failing op left no change"
    );
    assert_eq!(store.steps().len(), 1);
    store.undo().unwrap();
    assert_eq!(store.snapshot().doc(), &Cells::default());
    let head = store.snapshot();
    assert!(store.run(&caller, &Batch::atomic(ops)).is_err());
    assert!(Arc::ptr_eq(&head, &store.snapshot()));
}

struct Nothing;
impl Command<Cells> for Nothing {
    type Out = i64;
    type Error = Fail;
    fn validate(&self, _: &Cells) -> Result<(), Fail> {
        Ok(())
    }
    fn prepare(&self, tx: &mut Txn<'_, Cells>) -> Result<i64, Fail> {
        tx.read(1);
        Ok(tx.doc().get(1))
    }
}

#[test]
fn a_no_op_makes_no_version_and_no_step() {
    let store = fresh();
    put(&store, "a", &[(1, 4)]);
    let done = store.run(&Caller::anonymous(), &Nothing).unwrap();
    assert!(!done.changed);
    assert_eq!((done.out, done.version.seq()), (4, Seq(1)));
    assert_eq!(store.steps().len(), 1);
    assert!(matches!(
        store.run(
            &Caller::anonymous(),
            &Mix {
                reads: vec![],
                write: BAD - 1,
                add: 0,
                dawdle: false
            }
        ),
        Err(Fail::Refused)
    ));
}

#[test]
fn numbers_and_ids_go_on_after_a_reopen() {
    let store = fresh();
    let caller = Caller::anonymous();
    let mix = Mix {
        reads: vec![],
        write: 1,
        add: 1,
        dawdle: false,
    };
    for _ in 0..3 {
        store.run(&caller, &mix).unwrap();
    }
    store.undo().unwrap();
    let saved = store.snapshot();
    let counters = store.counters(&saved);
    assert_eq!(counters.to_string(), "ooxml-store.1.4.4.1:3");
    let text = counters.to_string();
    let reopened = Store::reopen(
        saved.doc().clone(),
        StoreConfig::default(),
        still(),
        &text.parse::<Counters>().unwrap(),
    )
    .unwrap();
    assert_eq!(reopened.snapshot().seq(), Seq(4));
    let done = reopened.run(&caller, &mix).unwrap();
    assert_eq!((done.version.seq(), done.out.1), (Seq(5), 4));
    let bad = Counters {
        version: Seq(9),
        last: Seq(3),
        ..Counters::default()
    };
    assert!(Store::reopen(Cells::default(), StoreConfig::default(), still(), &bad).is_err());
    let _ = IdSeq(0);
}

/// Prepares by first committing another change through the same store (so the head moves
/// while this transaction is open), then writes `write` after reading `read`.
struct Racing<'s> {
    store: &'s Store<Cells>,
    inner: (u32, i64),
    read: u32,
    write: u32,
}
impl Command<Cells> for Racing<'_> {
    type Out = ();
    type Error = Fail;
    fn validate(&self, _: &Cells) -> Result<(), Fail> {
        Ok(())
    }
    fn prepare(&self, tx: &mut Txn<'_, Cells>) -> Result<(), Fail> {
        if tx.base().seq() == Seq(0) {
            self.store.run(&Caller::named("other"), &Put(vec![self.inner]))?;
        }
        tx.read(self.read);
        let value = tx.doc().get(self.read) + 1;
        tx.write(Change::Set(self.write, Some(value)))?;
        Ok(())
    }
}

#[test]
fn a_moved_head_takes_disjoint_changes_and_redoes_conflicting_ones() {
    let store = fresh();
    let done = store
        .run(
            &Caller::anonymous(),
            &Racing {
                store: &store,
                inner: (5, 50),
                read: 1,
                write: 2,
            },
        )
        .unwrap();
    assert_eq!(done.attempts, 1, "disjoint: no redo");
    assert_eq!(
        (done.version.seq(), done.version.doc().get(5), done.version.doc().get(2)),
        (Seq(2), 50, 1)
    );
    let store2 = Store::new(Cells::default(), StoreConfig::default(), still());
    let done = store2
        .run(
            &Caller::anonymous(),
            &Racing {
                store: &store2,
                inner: (1, 70),
                read: 1,
                write: 2,
            },
        )
        .unwrap();
    assert_eq!(done.attempts, 2, "it read what the other wrote: redone");
    assert_eq!(done.version.doc().get(2), 71);
    assert_eq!(store2.stats().conflicts, 1);
}

#[test]
fn a_whole_document_change_conflicts_with_everything_and_undoes() {
    let store = fresh();
    put(&store, "a", &[(1, 1)]);
    let all: Map = Arc::new([(9, 9)].into_iter().collect());
    struct Replace(Map);
    impl Command<Cells> for Replace {
        type Out = ();
        type Error = Fail;
        fn validate(&self, _: &Cells) -> Result<(), Fail> {
            Ok(())
        }
        fn prepare(&self, tx: &mut Txn<'_, Cells>) -> Result<(), Fail> {
            tx.read_everything();
            Ok(tx.write(Change::All(self.0.clone()))?)
        }
    }
    let done = store.run(&Caller::anonymous(), &Replace(all)).unwrap();
    assert_eq!(done.writes, KeySet::Everything);
    assert_eq!(done.version.last_written(&1), done.version.seq());
    let first = store.steps()[0].step;
    assert!(matches!(
        store.undo_step(first, &KeySet::new()),
        Err(StoreError::Entangled { .. })
    ));
    store.undo().unwrap();
    assert_eq!(store.snapshot().doc().get(1), 1);
}
