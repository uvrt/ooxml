//! Owns: [`Version`], one immutable state of a document with its number and, per key, the
//! number of the version that last wrote it (what conflict detection and the entanglement
//! rules compare); [`Seq`], a version's number.
//!
//! Must not: change a version once it is published; everything here is persistent.

use crate::stamps::Stamps;
use crate::{DocModel, KeySet};
use std::fmt;
use std::sync::Arc;

/// A version's number. Monotonic and never given twice, also across a reopen
/// ([`crate::Counters`]); an undo makes a new number whose content is an earlier version's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Seq(pub u64);

impl Seq {
    /// The number after this one.
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for Seq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A published version, shared by every reader that holds it.
pub type Snapshot<D> = Arc<Version<D>>;

/// One immutable state of a document.
#[derive(Debug)]
pub struct Version<D: DocModel> {
    seq: Seq,
    doc: D,
    stamps: Stamps<D::Key>,
}

impl<D: DocModel> Version<D> {
    pub(crate) fn first(seq: Seq, doc: D) -> Self {
        Self {
            seq,
            doc,
            stamps: Stamps::default(),
        }
    }

    /// The version after `self`: `doc` at `seq`, with `writes` stamped.
    pub(crate) fn succeed(&self, seq: Seq, doc: D, writes: &KeySet<D::Key>) -> Self {
        let mut stamps = self.stamps.clone();
        stamps.stamp(writes, seq);
        Self { seq, doc, stamps }
    }

    /// The version's number.
    pub fn seq(&self) -> Seq {
        self.seq
    }

    /// The document.
    pub fn doc(&self) -> &D {
        &self.doc
    }

    /// The number of the last version that wrote `key`; `Seq(0)` when none did since the store
    /// opened.
    pub fn last_written(&self, key: &D::Key) -> Seq {
        self.stamps.of(key)
    }

    /// Whether a key of `set` was written by a version after `since`.
    pub fn written_since(&self, since: Seq, set: &KeySet<D::Key>) -> bool {
        if self.seq <= since {
            return false;
        }
        match set.keys() {
            None => true,
            Some(keys) => self.stamps.everything() > since || keys.iter().any(|key| self.stamps.of(key) > since),
        }
    }

    /// The keys of `set` written by a version after `since` (every key when `set` is every
    /// key and anything was written since).
    pub fn keys_written_since(&self, since: Seq, set: &KeySet<D::Key>) -> KeySet<D::Key> {
        match set.keys() {
            None if self.seq > since => KeySet::Everything,
            None => KeySet::new(),
            Some(keys) => KeySet::of(keys.iter().filter(|key| self.stamps.of(key) > since).cloned()),
        }
    }
}
