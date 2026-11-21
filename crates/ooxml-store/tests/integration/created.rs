//! Keys created under stand-ins (design section 5, lane D1): two transactions that each
//! append a row named by the next number, both prepared on one snapshot, draw the same
//! stand-in; the number is drawn again where each lands, so both commit without a conflict,
//! the later one taking the next number as a serial run would. What they created is stamped,
//! kept in the history and journaled under the numbers drawn, never under the stand-in: a
//! read of the first one's row conflicts with neither's stand-in, and a later write of a row
//! entangles the step that made it.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::still;
use ooxml_store::{Caller, Command, DocModel, Error, KeySet, Store, StoreConfig, Txn};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Rows, each a number and a value; the next number to give; what the transaction that made
/// this version drew for its stand-ins.
#[derive(Debug, Clone, Default)]
struct Rows {
    rows: Arc<BTreeMap<u64, i64>>,
    next: u64,
    drawn: Arc<BTreeMap<u64, u64>>,
}

#[derive(Debug, Clone)]
enum Change {
    /// A row with the next number, its stand-in the number the version prepared on gave.
    Append { stand_in: u64, value: i64 },
    /// A row's value set.
    Set(u64, i64),
    /// The value set of a row this transaction appended, named by its stand-in.
    SetOwn(u64, i64),
}

#[derive(Debug, thiserror::Error)]
#[error("no row {0}")]
struct NoRow(u64);

/// The key of the numbers given (merged: drawing is no conflict).
const NEXT: u64 = 0;

impl DocModel for Rows {
    type Key = u64;
    type Change = Change;
    type Error = NoRow;

    fn apply(&mut self, change: &Change) -> Result<(), NoRow> {
        match change {
            Change::Append { stand_in, value } => {
                self.next += 1;
                let number = self.next;
                Arc::make_mut(&mut self.rows).insert(number, *value);
                Arc::make_mut(&mut self.drawn).insert(*stand_in, number);
            }
            Change::Set(row, value) => {
                let slot = Arc::make_mut(&mut self.rows).get_mut(row).ok_or(NoRow(*row))?;
                *slot = *value;
            }
            Change::SetOwn(stand_in, value) => {
                let row = *self.drawn.get(stand_in).ok_or(NoRow(*stand_in))?;
                Arc::make_mut(&mut self.rows).insert(row, *value);
            }
        }
        Ok(())
    }

    fn inverse(&self, change: &Change) -> Result<Change, NoRow> {
        match change {
            Change::Set(row, _) => Ok(Change::Set(*row, *self.rows.get(row).ok_or(NoRow(*row))?)),
            Change::Append { .. } | Change::SetOwn(..) => Err(NoRow(0)),
        }
    }

    fn agrees(&self, other: &Rows, keys: &KeySet<u64>) -> bool {
        match keys.keys() {
            None => self.rows == other.rows,
            Some(keys) => keys.iter().all(|k| self.rows.get(k) == other.rows.get(k)),
        }
    }

    fn written(change: &Change, into: &mut KeySet<u64>) {
        match change {
            Change::Append { .. } => into.insert(NEXT),
            Change::Set(row, _) => into.insert(*row),
            Change::SetOwn(..) => {}
        }
    }

    fn merged(change: &Change, into: &mut KeySet<u64>) {
        if let Change::Append { .. } = change {
            into.insert(NEXT);
        }
    }

    fn kept(key: &u64) -> bool {
        *key == NEXT
    }

    fn created(change: &Change, into: &mut KeySet<u64>) {
        if let Change::Append { stand_in, .. } | Change::SetOwn(stand_in, _) = change {
            into.insert(*stand_in);
        }
    }

    fn resolve(&self, created: &KeySet<u64>) -> KeySet<u64> {
        KeySet::of(
            created
                .keys()
                .into_iter()
                .flatten()
                .map(|k| self.drawn.get(k).copied().unwrap_or(*k)),
        )
    }

    fn begin(&mut self) {
        self.drawn = Arc::default();
    }
}

/// Append a row (and set it, as a batch writing its own new row would); before its first
/// commit, `meanwhile` is committed by another caller (once).
struct Append<'s> {
    value: i64,
    store: &'s Store<Rows>,
    meanwhile: Option<Box<Append<'s>>>,
    done: AtomicBool,
}

impl Command<Rows> for Append<'_> {
    type Out = u64;
    type Error = Error<Rows>;

    fn validate(&self, _doc: &Rows) -> Result<(), Error<Rows>> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, Rows>) -> Result<u64, Error<Rows>> {
        let stand_in = tx.doc().next + 1;
        tx.write(Change::Append {
            stand_in,
            value: self.value,
        })?;
        // its own new row, read and written again in the same transaction
        tx.read(stand_in);
        tx.write(Change::SetOwn(stand_in, self.value + 1))?;
        if let Some(other) = &self.meanwhile {
            if !self.done.swap(true, Ordering::SeqCst) {
                self.store.run(&Caller::named("other"), &**other)?;
            }
        }
        Ok(stand_in)
    }
}

fn append<'s>(store: &'s Store<Rows>, value: i64, meanwhile: Option<Append<'s>>) -> Append<'s> {
    Append {
        value,
        store,
        meanwhile: meanwhile.map(Box::new),
        done: AtomicBool::new(false),
    }
}

#[test]
fn two_appends_from_one_snapshot_both_commit_with_the_serial_numbers() {
    let store = Store::new(Rows::default(), StoreConfig::default(), still());
    let first = append(&store, 10, None);
    let second = append(&store, 20, Some(first));
    let done = store.run(&Caller::named("second"), &second).unwrap();
    assert_eq!(done.attempts, 1, "a draw where the change lands is no conflict");
    assert!(!done.exclusive);
    // the first committed row 1 for real; the second drew 1 as its stand-in and landed as 2
    let head = store.snapshot();
    assert_eq!(*head.doc().rows, BTreeMap::from([(1, 11), (2, 21)]));
    // stamped, kept and journaled under the number drawn, not the stand-in
    assert!(
        done.writes.contains(&2) && !done.writes.contains(&1),
        "{:?}",
        done.writes
    );
    let steps = store.steps();
    let last = steps.last().unwrap();
    assert!(last.writes.contains(&2) && !last.writes.contains(&1));
    assert!(!last.reads.contains(&1), "its read of its own stand-in is not kept");
    assert_eq!(head.last_written(&1), steps[0].step);
    assert_eq!(head.last_written(&2), last.step);
}

#[test]
fn a_later_write_of_a_new_row_entangles_the_step_that_made_it() {
    struct Set(u64, i64);
    impl Command<Rows> for Set {
        type Out = ();
        type Error = Error<Rows>;
        fn validate(&self, _doc: &Rows) -> Result<(), Error<Rows>> {
            Ok(())
        }
        fn prepare(&self, tx: &mut Txn<'_, Rows>) -> Result<(), Error<Rows>> {
            tx.read(self.0);
            tx.write(Change::Set(self.0, self.1))
        }
    }
    let store = Store::new(Rows::default(), StoreConfig::default(), still());
    let first = append(&store, 10, None);
    let second = append(&store, 20, Some(first));
    let made = store.run(&Caller::named("second"), &second).unwrap();
    store.run(&Caller::named("third"), &Set(2, 99)).unwrap();
    // the second's row is 2 (resolved): the third's write of it entangles that step
    assert!(store.undo_step(made.version.seq(), &KeySet::new()).is_err());
}
