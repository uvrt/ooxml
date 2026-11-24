//! Owns: each table's rows' keeps ([`super::keeps::row_keeps`]) kept across a document's
//! versions, in the block tier beside the tables' readings ([`super::cells`]): by the table
//! element, the outermost table holding it (whose reading gives its paragraphs' cells) and
//! the formatting parts, all by identity and held weakly -- a table no edit touched keeps
//! its rows' keeps, wherever it moved. On the document's budget, least recently used let
//! go first.
//!
//! Must not: key by a path or an id (a row's deciding paragraph is named by its steps below
//! the table), or keep a table's rows' keeps alive past the versions holding its element.

use super::cells::{held_roots, same_roots, HeldRoots};
use super::formatting::Formatting;
use super::keeps::RowKeep;
use ooxml_derive::{AddressHash, CacheBudget, Limit, Memo, MemoStats, Weigh};
use ooxml_xml::Element;
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// What the rows' keeps hold at most where no budget is given, bytes.
pub const ROWS_MEMO_BYTES: usize = 8 << 20;

/// One table's rows' keeps, and the elements they were read from, held weakly.
#[derive(Debug, Clone)]
struct KeptRows {
    table: Weak<Element>,
    outer: Weak<Element>,
    roots: HeldRoots,
    rows: Arc<[RowKeep]>,
}

impl KeptRows {
    /// Whether they were read from `table` in `outer` with `formatting`'s parts.
    fn of(&self, table: &Arc<Element>, outer: &Arc<Element>, formatting: &Formatting) -> bool {
        std::ptr::eq(self.table.as_ptr(), Arc::as_ptr(table))
            && std::ptr::eq(self.outer.as_ptr(), Arc::as_ptr(outer))
            && same_roots(&self.roots, formatting)
    }

    /// Whether a version can still ask for them: their table element is held.
    fn alive(&self) -> bool {
        self.table.strong_count() > 0 && self.outer.strong_count() > 0
    }
}

impl Weigh for KeptRows {
    fn weight(&self) -> usize {
        let leads: usize = self
            .rows
            .iter()
            .map(|r| r.lead.as_ref().map_or(0, |l| 24 + 4 * l.len()))
            .sum();
        std::mem::size_of::<KeptRows>() + self.rows.len() * std::mem::size_of::<RowKeep>() + leads
    }
}

/// Each table's rows' keeps kept across a document's versions, by the table element's
/// address (a weak reference keeps the address its own while it is kept), and how many
/// were alive when last swept.
pub(crate) struct RowsMemo {
    memo: Mutex<(Memo<usize, KeptRows, AddressHash>, usize)>,
}

impl std::fmt::Debug for RowsMemo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("RowsMemo").field("stats", &memo.0.stats()).finish()
    }
}

impl Default for RowsMemo {
    fn default() -> RowsMemo {
        RowsMemo::new(Arc::new(Limit::new(ROWS_MEMO_BYTES)))
    }
}

impl RowsMemo {
    /// An empty memo counted on `budget`.
    pub(crate) fn new(budget: Arc<dyn CacheBudget>) -> RowsMemo {
        RowsMemo {
            memo: Mutex::new((Memo::with_hasher(budget), 0)),
        }
    }

    /// What it holds.
    pub(crate) fn stats(&self) -> MemoStats {
        self.memo.lock().unwrap_or_else(PoisonError::into_inner).0.stats()
    }

    /// The rows' keeps of `table` (in `outer`, the outermost table holding it, itself where
    /// it is not nested) with `formatting`: as kept, else made by `read` and kept.
    pub(crate) fn rows(
        &self,
        formatting: &Formatting,
        table: &Arc<Element>,
        outer: &Arc<Element>,
        read: impl FnOnce() -> Vec<RowKeep>,
    ) -> Arc<[RowKeep]> {
        let key = Arc::as_ptr(table) as usize;
        let found = {
            let mut memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
            memo.0
                .get(&key)
                .filter(|k| k.of(table, outer, formatting))
                .map(|k| Arc::clone(&k.rows))
        };
        if let Some(found) = found {
            return found;
        }
        // Read outside the lock: another reader of another table is not held up.
        let rows: Arc<[RowKeep]> = Arc::from(read());
        let kept = KeptRows {
            table: Arc::downgrade(table),
            outer: Arc::downgrade(outer),
            roots: held_roots(formatting),
            rows: Arc::clone(&rows),
        };
        let mut memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
        let (kept_now, held) = &mut *memo;
        kept_now.insert(key, kept);
        // Keeps of tables no version holds: swept once the memo has doubled since.
        if kept_now.stats().entries > 2 * *held + 64 {
            kept_now.sweep(KeptRows::alive);
            *held = kept_now.stats().entries;
        }
        rows
    }
}
