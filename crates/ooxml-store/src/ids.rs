//! Owns: id sequences that never give a position twice ([`IdSeq`], the allocator a
//! transaction draws from), and [`Counters`], the state kept beside a saved document so
//! that version numbers and id sequences go on after a reopen.
//!
//! Must not: know what an id looks like. A sequence gives positions 1, 2, 3, ...; the
//! format maps a position to a value (Word's stamping sequence per seed, say). A position
//! drawn by a transaction that was redone or failed is never given again.

use crate::sync::{lock, Mutex};
use crate::{CountersError, Seq};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

/// A named sequence of ids (a seed, an id kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdSeq(pub u64);

/// What a store needs to go on after a reopen: the version the saved bytes are, the highest
/// version number given, and how far each id sequence got.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Counters {
    /// The version the saved document is; the store reopens at this number.
    pub version: Seq,
    /// The highest version number given; the next change gets one more.
    pub last: Seq,
    /// Per sequence, the last position given.
    pub cursors: BTreeMap<IdSeq, u64>,
}

/// The form's name and number; a later form changes the number and still reads this one.
const PREFIX: &str = "ooxml-store.1";

impl fmt::Display for Counters {
    /// `ooxml-store.1.<version>.<last>` then `.<sequence>:<position>` per sequence.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PREFIX}.{}.{}", self.version, self.last)?;
        for (seq, cursor) in &self.cursors {
            write!(f, ".{}:{cursor}", seq.0)?;
        }
        Ok(())
    }
}

impl FromStr for Counters {
    type Err = CountersError;

    fn from_str(text: &str) -> Result<Self, CountersError> {
        let rest = text
            .strip_prefix(PREFIX)
            .and_then(|rest| rest.strip_prefix('.'))
            .ok_or_else(|| CountersError::Form(text.to_owned()))?;
        let mut fields = rest.split('.');
        let number = |field: Option<&str>| -> Result<u64, CountersError> {
            field
                .and_then(|f| f.parse().ok())
                .ok_or_else(|| CountersError::Form(text.to_owned()))
        };
        let version = Seq(number(fields.next())?);
        let last = Seq(number(fields.next())?);
        let mut cursors = BTreeMap::new();
        for field in fields {
            let (seq, cursor) = field
                .split_once(':')
                .ok_or_else(|| CountersError::Form(text.to_owned()))?;
            let seq = IdSeq(number(Some(seq))?);
            if cursors.insert(seq, number(Some(cursor))?).is_some() {
                return Err(CountersError::Form(text.to_owned()));
            }
        }
        let counters = Self { version, last, cursors };
        counters.check()?;
        Ok(counters)
    }
}

impl Counters {
    /// Refuse counters whose version is after the last number given.
    pub fn check(&self) -> Result<(), CountersError> {
        if self.version > self.last {
            return Err(CountersError::VersionAfterLast {
                version: self.version,
                last: self.last,
            });
        }
        Ok(())
    }
}

/// Where every sequence got, and the version numbers ([`Seq`]): the highest given. Its own
/// locks, never held while taking the commit lock, so a transaction draws ids -- and a number
/// for a version it makes but does not publish ([`crate::Txn::version_here`]) -- while it
/// prepares without waiting for commits; a publish draws its number under the commit lock.
pub(crate) struct IdAllocator {
    cursors: Mutex<BTreeMap<IdSeq, u64>>,
    /// The highest version number given.
    versions: Mutex<Seq>,
}

impl IdAllocator {
    pub(crate) fn new(cursors: BTreeMap<IdSeq, u64>, last: Seq) -> Self {
        Self {
            cursors: Mutex::new(cursors),
            versions: Mutex::new(last),
        }
    }

    /// A version number never given before.
    pub(crate) fn next_seq(&self) -> Seq {
        let mut last = lock(&self.versions);
        *last = last.next();
        *last
    }

    /// The highest version number given.
    pub(crate) fn last_seq(&self) -> Seq {
        *lock(&self.versions)
    }

    /// The next position of `seq`.
    pub(crate) fn draw(&self, seq: IdSeq) -> Result<u64, CountersError> {
        let mut cursors = lock(&self.cursors);
        let cursor = cursors.entry(seq).or_insert(0);
        *cursor = cursor.checked_add(1).ok_or(CountersError::Exhausted(seq))?;
        Ok(*cursor)
    }

    pub(crate) fn cursors(&self) -> BTreeMap<IdSeq, u64> {
        lock(&self.cursors).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_round_trip_and_refuse_damage() {
        let mut counters = Counters {
            version: Seq(7),
            last: Seq(12),
            cursors: BTreeMap::new(),
        };
        counters.cursors.insert(IdSeq(3), 40);
        counters.cursors.insert(IdSeq(99), 1);
        let text = counters.to_string();
        assert_eq!(text, "ooxml-store.1.7.12.3:40.99:1");
        assert_eq!(text.parse::<Counters>().unwrap(), counters);
        for bad in [
            "ooxml-store.2.1.1",
            "ooxml-store.1.1",
            "ooxml-store.1.9.3",
            "ooxml-store.1.1.1.3:4.3:5",
            "x",
        ] {
            assert!(bad.parse::<Counters>().is_err(), "{bad}");
        }
    }
}
