//! A made-up model for the store's tests: numbered cells holding numbers, a change sets one
//! cell. Plus a seeded random source and clocks.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
#![allow(dead_code)]

use ooxml_store::{Clock, Command, DocModel, Error, IdSeq, KeySet, Txn};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cells(pub Map);

/// The cells, shared until written (the tests' documents are small).
pub type Map = Arc<BTreeMap<u32, i64>>;

impl Cells {
    pub fn get(&self, cell: u32) -> i64 {
        self.0.get(&cell).copied().unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Set a cell (`None` empties it).
    Set(u32, Option<i64>),
    /// Replace every cell.
    All(Map),
}

/// The cell no change may write.
pub const BAD: u32 = u32::MAX;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("cell {0} cannot be written")]
pub struct Refused(pub u32);

impl DocModel for Cells {
    type Key = u32;
    type Change = Change;
    type Error = Refused;
    type Setup = ();
    /// The cells that differ: each one's value before and after.
    type Delta = BTreeMap<u32, (i64, i64)>;

    fn delta(before: &Cells, after: &Cells) -> BTreeMap<u32, (i64, i64)> {
        before
            .0
            .keys()
            .chain(after.0.keys())
            .filter(|cell| before.get(**cell) != after.get(**cell))
            .map(|cell| (*cell, (before.get(*cell), after.get(*cell))))
            .collect()
    }

    fn apply(&mut self, change: &Change) -> Result<(), Refused> {
        match change {
            Change::Set(BAD, _) => return Err(Refused(BAD)),
            Change::Set(cell, Some(value)) => {
                Arc::make_mut(&mut self.0).insert(*cell, *value);
            }
            Change::Set(cell, None) => {
                Arc::make_mut(&mut self.0).remove(cell);
            }
            Change::All(cells) => self.0 = cells.clone(),
        }
        Ok(())
    }

    fn inverse(&self, change: &Change) -> Result<Change, Refused> {
        Ok(match change {
            Change::Set(cell, _) => Change::Set(*cell, self.0.get(cell).copied()),
            Change::All(_) => Change::All(self.0.clone()),
        })
    }

    fn agrees(&self, other: &Cells, keys: &KeySet<u32>) -> bool {
        match keys.keys() {
            None => self == other,
            Some(keys) => keys.iter().all(|cell| self.0.get(cell) == other.0.get(cell)),
        }
    }

    fn written(change: &Change, into: &mut KeySet<u32>) {
        match change {
            Change::Set(cell, _) => into.insert(*cell),
            Change::All(_) => into.insert_everything(),
        }
    }
}

/// The test's command error: the store's, or a refusal of its own.
#[derive(Debug)]
pub enum Fail {
    Store(Error<Cells>),
    Refused,
}

impl From<Error<Cells>> for Fail {
    fn from(error: Error<Cells>) -> Self {
        Fail::Store(error)
    }
}

/// Read `reads`, then set `write` to a function of what was read and `add`; draws one id.
/// Returns the sum read and the id drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mix {
    pub reads: Vec<u32>,
    pub write: u32,
    pub add: i64,
    /// Yield while preparing, to widen the window for conflicts.
    pub dawdle: bool,
}

impl Command<Cells> for Mix {
    type Out = (i64, u64);
    type Error = Fail;

    fn validate(&self, _doc: &Cells) -> Result<(), Fail> {
        if self.write == BAD - 1 {
            return Err(Fail::Refused);
        }
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, Cells>) -> Result<(i64, u64), Fail> {
        let mut sum = 0i64;
        for &cell in &self.reads {
            tx.read(cell);
            sum = sum.wrapping_add(tx.doc().get(cell));
            if self.dawdle {
                std::thread::yield_now();
            }
        }
        let id = tx.allocate(IdSeq(1))?;
        tx.write(Change::Set(
            self.write,
            Some(sum.wrapping_mul(31).wrapping_add(self.add)),
        ))?;
        Ok((sum, id))
    }

    /// Nothing: it refuses on its arguments alone (the cell it writes).
    fn read_refusal(&self, _tx: &mut Txn<'_, Cells>) {}
}

/// Set `write` to `value` where cell `on` holds something; refused where it is empty. Its
/// refusal rests on `on` alone ([`Command::read_refusal`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guard {
    pub on: u32,
    pub write: u32,
    pub value: i64,
}

impl Command<Cells> for Guard {
    type Out = ();
    type Error = Fail;

    fn validate(&self, doc: &Cells) -> Result<(), Fail> {
        if doc.get(self.on) == 0 {
            return Err(Fail::Refused);
        }
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, Cells>) -> Result<(), Fail> {
        tx.read(self.on);
        tx.write(Change::Set(self.write, Some(self.value)))?;
        Ok(())
    }

    fn read_refusal(&self, tx: &mut Txn<'_, Cells>) {
        tx.read(self.on);
    }
}

/// Set cells to values without reading anything.
#[derive(Debug, Clone)]
pub struct Put(pub Vec<(u32, i64)>);

impl Command<Cells> for Put {
    type Out = ();
    type Error = Fail;

    fn validate(&self, _doc: &Cells) -> Result<(), Fail> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, Cells>) -> Result<(), Fail> {
        for &(cell, value) in &self.0 {
            tx.write(Change::Set(cell, Some(value)))?;
        }
        Ok(())
    }
}

/// SplitMix64: a seeded source of test randomness.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A clock that stands still.
pub struct Still;
impl Clock for Still {
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

/// The system's monotonic clock.
pub struct System(pub Instant);
impl Clock for System {
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

pub fn still() -> Arc<dyn Clock> {
    Arc::new(Still)
}
