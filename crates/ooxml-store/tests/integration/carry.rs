//! What no step takes back stays across undo and redo ([`DocModel::carry`]): a made-up
//! document whose ledger of values ever written only grows, whatever undo and redo do to
//! its content.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common;

use common::{still, Rng};
use ooxml_store::{Caller, Command, DocModel, Error, KeySet, Store, StoreConfig, Txn};
use std::collections::BTreeSet;
use std::sync::Arc;

/// One value, and every value it ever held.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Ledgered {
    value: i64,
    ledger: Arc<BTreeSet<i64>>,
}

#[derive(Debug, thiserror::Error)]
#[error("never")]
struct Never;

impl DocModel for Ledgered {
    type Key = ();
    type Change = i64;
    type Error = Never;
    type Setup = ();
    type Delta = ();

    fn delta(_: &Self, _: &Self) {}

    fn apply(&mut self, change: &i64) -> Result<(), Never> {
        self.value = *change;
        Arc::make_mut(&mut self.ledger).insert(*change);
        Ok(())
    }

    fn inverse(&self, _change: &i64) -> Result<i64, Never> {
        Ok(self.value)
    }

    fn agrees(&self, other: &Self, _keys: &KeySet<()>) -> bool {
        self.value == other.value
    }

    fn written(_change: &i64, into: &mut KeySet<()>) {
        into.insert(());
    }

    fn carry(&mut self, head: &Self) {
        let mut ledger = (*self.ledger).clone();
        ledger.extend(head.ledger.iter().copied());
        self.ledger = Arc::new(ledger);
    }
}

struct Set(i64);

impl Command<Ledgered> for Set {
    type Out = ();
    type Error = Error<Ledgered>;

    fn validate(&self, _doc: &Ledgered) -> Result<(), Error<Ledgered>> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, Ledgered>) -> Result<(), Error<Ledgered>> {
        tx.read(());
        tx.write(self.0)
    }
}

#[test]
fn the_ledger_only_grows_across_undo_and_redo() {
    for seed in 0..10u64 {
        let mut rng = Rng(seed);
        let store = Store::new(Ledgered::default(), StoreConfig::default(), still());
        let mut given = BTreeSet::new();
        let mut before = BTreeSet::new();
        for step in 0..200i64 {
            match rng.below(4) {
                0 | 1 => {
                    store.run(&Caller::named("w"), &Set(step + 1)).unwrap();
                    given.insert(step + 1);
                }
                2 => drop(store.undo()),
                _ => drop(store.redo()),
            }
            let head = store.snapshot();
            let ledger = (*head.doc().ledger).clone();
            assert!(
                given.is_subset(&ledger),
                "seed {seed} step {step}: {given:?} {ledger:?}"
            );
            assert!(before.is_subset(&ledger), "seed {seed} step {step}: shrank");
            before = ledger;
        }
    }
}
