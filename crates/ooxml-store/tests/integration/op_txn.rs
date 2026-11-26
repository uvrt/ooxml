//! A host's batch as Phase 3 runs it (`Store::run_txn`): each op handed the transaction
//! erased (`OpTxn`), reading the versions the ops before it left (`Txn::version_here`, never
//! published), a failed op taken back at its savepoint; prepared again from scratch on a
//! conflict, one commit and one step whichever attempt published.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common;

use common::{still, Cells, Change, Fail, Put};
use ooxml_store::{AttemptState, Caller, InOp, OpTxn, Store, StoreConfig, Txn};
use std::cell::Cell;
use std::sync::Arc;

/// Op: add `add` to `cell`, read through the erased transaction; `None` refuses.
fn op(txn: &dyn OpTxn, cell: u32, add: Option<i64>) -> Result<i64, Fail> {
    txn.with::<Cells, _>(|tx, current| {
        let Some(add) = add else {
            tx.write(Change::Set(cell, Some(-1)))?;
            return Err(Fail::Refused);
        };
        tx.read(cell);
        let was = current.doc().get(cell);
        tx.write(Change::Set(cell, Some(was + add)))?;
        *current = tx.version_here(current);
        Ok(was + add)
    })
    .unwrap()
}

/// The batch's ops in `tx`, each at a savepoint, as the session runs them.
fn ops(tx: &mut Txn<'static, Cells>, plan: &[(u32, Option<i64>)]) -> Result<Vec<Option<i64>>, Fail> {
    let attempt = AttemptState::default();
    let mut current = Arc::clone(tx.base());
    let mut outs = Vec::new();
    for (cell, add) in plan {
        let point = tx.savepoint();
        let lent = InOp::new(tx, Arc::clone(&current), &attempt);
        let ran = op(&lent, *cell, *add);
        let left = lent.into_current::<Cells>().unwrap();
        match ran {
            Ok(value) => {
                current = left;
                outs.push(Some(value));
            }
            Err(_) => {
                tx.rollback(point);
                outs.push(None);
            }
        }
    }
    Ok(outs)
}

#[test]
fn each_op_reads_the_version_the_ops_before_it_left_and_the_batch_is_one_step() {
    let store = Store::new(Cells::default(), StoreConfig::default(), still());
    let plan = [(1, Some(5)), (1, Some(2)), (2, None), (3, Some(1))];
    let done = store
        .run_txn(&Caller::named("batch"), &(), |tx| ops(tx, &plan))
        .unwrap();
    assert_eq!(done.out, [Some(5), Some(7), None, Some(1)], "op 1 read op 0's version");
    assert_eq!(store.snapshot().doc().get(1), 7);
    assert_eq!(store.snapshot().doc().get(2), 0, "the refusing op taken back alone");
    assert_eq!(store.steps().len(), 1);
    assert_eq!(done.version.number(), 1);
}

#[test]
fn a_conflict_prepares_the_batch_again_from_scratch_on_the_new_head() {
    let store = Store::new(Cells::default(), StoreConfig::default(), still());
    let raced = Cell::new(false);
    let done = store
        .run_txn(&Caller::named("batch"), &(), |tx| {
            let outs = ops(tx, &[(1, Some(5))]);
            if !raced.replace(true) {
                // a writer commits the cell the batch read, between its prepare and its commit
                store.run(&Caller::named("other"), &Put(vec![(1, 10)])).unwrap();
            }
            outs
        })
        .unwrap();
    assert_eq!(done.attempts, 2);
    assert_eq!(done.out, [Some(15)], "the second attempt read the writer's value");
    assert_eq!(store.snapshot().doc().get(1), 15, "no update lost");
    assert_eq!(store.steps().len(), 2);
}
