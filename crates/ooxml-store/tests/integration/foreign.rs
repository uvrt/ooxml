//! Another editor's commits (co-authoring refresh, `Store::receive`): a transaction prepared
//! before one that changed what it read is refused, not prepared again; a disjoint one is
//! rebased past it; the steps since its base that touch what it changed are handed back by
//! scoped undos, with every step entangled with them; plain undo and redo never take back
//! another editor's step and pass one only where they are not entangled with it.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common;

use common::{still, Cells, Change, Fail, Mix, Put};
use ooxml_store::{Caller, KeySet, Prepared, Received, Seq, StepInfo, Store, StoreConfig, StoreError, Txn};
use std::cell::Cell;

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

/// Another editor's commit of `cells`, made on `base`: it touches a step that wrote one of
/// them.
fn receive(store: &Store<Cells>, base: Seq, cells: &[(u32, i64)]) -> Result<Received<Cells>, Fail> {
    let keys = KeySet::of(cells.iter().map(|(cell, _)| *cell));
    store.receive(
        &Caller::foreign("Ada"),
        base,
        &(),
        |step: &StepInfo<u32>| step.writes.intersects(&keys),
        |tx: &mut Txn<'_, Cells>, _base| {
            for &(cell, value) in cells {
                tx.read(cell);
                tx.write(Change::Set(cell, Some(value)))?;
            }
            Ok(Prepared::Made)
        },
    )
}

/// A transaction reading `reads` and writing cell 9, during whose first preparation another
/// editor commits `theirs`.
fn prepared_across(store: &Store<Cells>, reads: &[u32], theirs: &[(u32, i64)]) -> Result<u32, Fail> {
    let first = Cell::new(true);
    let base = store.snapshot().seq();
    store
        .run_txn(&Caller::named("agent"), &(), |tx| {
            for &cell in reads {
                tx.read(cell);
            }
            tx.write(Change::Set(9, Some(1)))?;
            if first.replace(false) {
                receive(store, base, theirs)?;
            }
            Ok(())
        })
        .map(|done| done.attempts)
}

#[test]
fn a_transaction_that_read_what_another_editor_changed_is_refused_naming_it() {
    let store = fresh();
    put(&store, "agent", &[(1, 1), (2, 2)]);
    let refused = prepared_across(&store, &[1], &[(1, 10)]);
    match refused {
        Err(Fail::Store(StoreError::ForeignChange { editor, overlap, .. })) => {
            assert_eq!(&*editor, "Ada");
            assert_eq!(overlap, KeySet::of([1]));
        }
        other => panic!("not refused: {other:?}"),
    }
    assert_eq!(store.snapshot().doc().get(1), 10);
    assert_eq!(store.snapshot().doc().get(9), 0, "nothing of the refused transaction");
}

#[test]
fn a_disjoint_transaction_is_rebased_past_another_editors_commit() {
    let store = fresh();
    put(&store, "agent", &[(1, 1), (2, 2)]);
    let attempts = prepared_across(&store, &[2], &[(1, 10)]).unwrap();
    assert_eq!(attempts, 1, "rebased, not prepared again");
    let head = store.snapshot();
    assert_eq!((head.doc().get(1), head.doc().get(9)), (10, 1));
}

#[test]
fn steps_since_the_base_that_touch_it_are_handed_back_with_those_entangled() {
    let store = fresh();
    let base = put(&store, "a", &[(5, 5)]);
    let one = put(&store, "a", &[(1, 1)]);
    let two = put(&store, "b", &[(2, 2)]);
    // reads cell 1, which `one` wrote: entangled with it
    let three = store
        .run(
            &Caller::named("c"),
            &Mix {
                reads: vec![1],
                write: 3,
                add: 0,
                dawdle: false,
            },
        )
        .unwrap()
        .version
        .seq();
    let received = receive(&store, base, &[(1, 10)]).unwrap();
    let back: Vec<Seq> = received.handed_back.iter().map(|h| h.step.step).collect();
    assert_eq!(back, vec![three, one], "newest first, the entangled one with it");
    assert_eq!(&*received.handed_back[1].step.caller.clone().unwrap(), "a");
    let head = store.snapshot();
    assert_eq!(head.doc().get(1), 10, "theirs stands");
    assert_eq!(head.doc().get(2), 2, "a disjoint step stays");
    assert_eq!(head.doc().get(3), 0);
    assert!(store.steps().iter().any(|s| s.step == two));
    let theirs = store.steps().last().cloned().unwrap();
    assert!(theirs.foreign);
    assert_eq!(theirs.caller.as_deref(), Some("Ada"));
}

#[test]
fn a_base_the_store_no_longer_keeps_is_refused() {
    let store = Store::new(
        Cells::default(),
        StoreConfig {
            history_depth: 2,
            ..StoreConfig::default()
        },
        still(),
    );
    let old = put(&store, "a", &[(1, 1)]);
    for v in 0..4 {
        put(&store, "a", &[(2, v)]);
    }
    assert!(matches!(
        receive(&store, Seq(old.0 - 1), &[(1, 10)]),
        Err(Fail::Store(StoreError::NotKept { .. }))
    ));
}

#[test]
fn plain_undo_passes_another_editors_disjoint_step_and_redo_brings_it_back() {
    let store = fresh();
    put(&store, "a", &[(1, 1)]);
    let base = store.snapshot().seq();
    receive(&store, base, &[(2, 20)]).unwrap();
    store.undo().unwrap();
    let head = store.snapshot();
    assert_eq!((head.doc().get(1), head.doc().get(2)), (0, 20));
    assert!(matches!(store.undo(), Err(StoreError::NothingToUndo)));
    store.redo().unwrap();
    assert_eq!(store.snapshot().doc().get(1), 1);
}

#[test]
fn plain_undo_entangled_with_another_editors_step_is_refused() {
    let store = fresh();
    put(&store, "a", &[(1, 1)]);
    let base = store.snapshot().seq();
    receive(&store, base, &[(1, 10)]).unwrap();
    match store.undo() {
        Err(StoreError::ForeignChange { editor, overlap, .. }) => {
            assert_eq!(&*editor, "Ada");
            assert_eq!(overlap, KeySet::of([1]));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(store.snapshot().doc().get(1), 10, "nothing changed");
}

#[test]
fn another_editors_commit_keeps_plain_redo() {
    let store = fresh();
    put(&store, "a", &[(1, 1)]);
    store.undo().unwrap();
    let base = store.snapshot().seq();
    receive(&store, base, &[(2, 20)]).unwrap();
    store.redo().unwrap();
    let head = store.snapshot();
    assert_eq!((head.doc().get(1), head.doc().get(2)), (1, 20));
    // a commit of this session's drops what plain redo kept, as it drops plain redo
    store.undo().unwrap();
    put(&store, "a", &[(3, 3)]);
    assert!(matches!(store.redo(), Err(StoreError::NothingToRedo)));
}

/// Add `value` to `cell` as one step by `who`: a merge where it lands.
struct Adding(u32, i64);

impl ooxml_store::Command<Cells> for Adding {
    type Out = ();
    type Error = Fail;

    fn validate(&self, _doc: &Cells) -> Result<(), Fail> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, Cells>) -> Result<(), Fail> {
        tx.write(Change::Add(self.0, self.1))?;
        Ok(())
    }
}

#[test]
fn steps_writing_a_key_only_as_merges_are_not_entangled_by_it() {
    let store = fresh();
    let first = store.run(&Caller::named("a"), &Adding(1, 5)).unwrap().version.seq();
    store.run(&Caller::named("b"), &Adding(1, 7)).unwrap();
    // the first taken back past the second: each only added to the cell
    store.undo_step(first, &KeySet::new()).unwrap();
    assert_eq!(store.snapshot().doc().get(1), 7);
    // a plain write of it after a merge is entangled, as before
    let merged = store.run(&Caller::named("a"), &Adding(2, 1)).unwrap().version.seq();
    put(&store, "b", &[(2, 9)]);
    assert!(matches!(
        store.undo_step(merged, &KeySet::new()),
        Err(StoreError::Entangled { .. })
    ));
}

#[test]
fn another_editors_commit_hands_back_only_what_touches_it_past_merges() {
    let store = fresh();
    let base = store.snapshot().seq();
    // both add to cell 9 (a shared merge, as people listed), then each writes its own cell
    let one = store.run(&Caller::named("a"), &Adding(9, 1)).unwrap().version.seq();
    put(&store, "a", &[(1, 1)]);
    store.run(&Caller::named("b"), &Adding(9, 1)).unwrap();
    let two = put(&store, "b", &[(2, 2)]);
    let received = receive(&store, base, &[(2, 20)]).unwrap();
    let back: Vec<Seq> = received.handed_back.iter().map(|h| h.step.step).collect();
    assert_eq!(back, vec![two], "the adds to the shared cell tie no step to another");
    let _ = one;
    let head = store.snapshot();
    assert_eq!((head.doc().get(1), head.doc().get(2), head.doc().get(9)), (1, 20, 2));
}
