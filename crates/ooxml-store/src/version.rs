//! Owns: [`Version`], one immutable state of a document with its number and, per key, the
//! number of the version that last wrote it (what conflict detection and the entanglement
//! rules compare); [`Seq`], a version's number; the state's number ([`Version::number`]),
//! the document's states numbered as a tool layer numbers them (the store gives it at
//! publish: [`crate::Store`]).
//!
//! Must not: change a version once it is published; everything here is persistent.

use crate::stamps::Stamps;
use crate::{DocModel, KeySet};
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

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
    number: u64,
    doc: D,
    stamps: Stamps<D::Key>,
    changes: Changes<D>,
}

/// What a version changed against the version it was published after, worked out when first
/// asked ([`Version::changes`]): until then the earlier version's document is held (the
/// document alone, never its version, so versions do not chain), and let go once it is.
struct Changes<D: DocModel> {
    before: Mutex<Option<D>>,
    worked: OnceLock<Option<Arc<D::Delta>>>,
}

impl<D: DocModel> Changes<D> {
    fn none() -> Self {
        Changes {
            before: Mutex::new(None),
            worked: OnceLock::from(None),
        }
    }

    fn against(before: D) -> Self {
        Changes {
            before: Mutex::new(Some(before)),
            worked: OnceLock::new(),
        }
    }

    fn get(&self, after: &D) -> Option<Arc<D::Delta>> {
        self.worked
            .get_or_init(|| {
                let before = self.before.lock().unwrap_or_else(PoisonError::into_inner).take()?;
                Some(Arc::new(D::delta(&before, after)))
            })
            .clone()
    }

    /// Whether the earlier version's document is still held.
    fn holding(&self) -> bool {
        self.before.lock().unwrap_or_else(PoisonError::into_inner).is_some()
    }
}

impl<D: DocModel> fmt::Debug for Changes<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Changes")
            .field("worked", &self.worked.get().is_some())
            .finish_non_exhaustive()
    }
}

impl<D: DocModel> Version<D> {
    pub(crate) fn first(seq: Seq, doc: D) -> Self {
        Self {
            seq,
            number: 0,
            doc,
            stamps: Stamps::default(),
            changes: Changes::none(),
        }
    }

    /// The same version, its state numbered `number` (a store going on with numbers given
    /// before it opened: [`crate::Store::continue_numbers`]); only a first version is
    /// renumbered, so it has no changes.
    pub(crate) fn renumbered(&self, number: u64) -> Self {
        Self {
            seq: self.seq,
            number,
            doc: self.doc.clone(),
            stamps: self.stamps.clone(),
            changes: Changes::none(),
        }
    }

    /// The version after `self`: `doc` at `seq`, the state numbered `number`, with `writes`
    /// stamped.
    pub(crate) fn succeed(&self, seq: Seq, number: u64, doc: D, writes: &KeySet<D::Key>) -> Self {
        let mut stamps = self.stamps.clone();
        stamps.stamp(writes, seq);
        Self {
            seq,
            number,
            doc,
            stamps,
            changes: Changes::against(self.doc.clone()),
        }
    }

    /// The same version, its changes worked out against `previous` (the document of the
    /// version a reader held before this one, where that is not the version it was made
    /// from: a held transaction's versions, made from its base).
    pub(crate) fn changed_from(mut self, previous: D) -> Self {
        self.changes = Changes::against(previous);
        self
    }

    /// The version's number.
    pub fn seq(&self) -> Seq {
        self.seq
    }

    /// The state's number, as a tool layer numbers a document's states: the state opened
    /// 0, a step one more than the highest number given, a plain undo or redo the number
    /// of the state it brings back (so numbers repeat, unlike [`Self::seq`]); see
    /// [`crate::Store`]'s numbering rule.
    pub fn number(&self) -> u64 {
        self.number
    }

    /// The document.
    pub fn doc(&self) -> &D {
        &self.doc
    }

    /// What this version changed against the version it was published after
    /// ([`DocModel::delta`]), worked out on the first ask and kept; `None` for a store's
    /// first version.
    pub fn changes(&self) -> Option<Arc<D::Delta>> {
        self.changes.get(&self.doc)
    }

    /// Whether this version still holds the earlier version's document its changes are
    /// worked out from (nobody has asked for them yet).
    pub fn holds_before(&self) -> bool {
        self.changes.holding()
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
