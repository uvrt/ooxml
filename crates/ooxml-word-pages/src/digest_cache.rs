//! Owns: each page's summary as digested ([`DigestCache`]), kept across a generation's
//! versions by the page as drawn -- what its `LaidPage` is drawn as, kept by a page copied
//! from it moved only in its flow's places (an insert before it), which draws the flow items
//! it drew, moved with it -- and its context ([`crate::page_context`]): a page another version lays out
//! the same, in the same context, is taken as digested, not painted again, where it draws
//! the same blocks under the same keys (a summary names its body lines by id, an id is its
//! block's key's: the keys are checked, never part of the memo's key, ADR-008). It holds its
//! pages within its own bounded account, the least recently used let go first; it goes
//! with its generation.
//!
//! What it keeps of a page is never the page: only weak references tell it again (a page
//! no version holds any more is never found again, and its memory is not held).
//!
//! Must not: paint, keep a laid page or flow item alive, or grow past its bound.

use crate::page_reads::PageBlocks;
use crate::summary::PageSummary;
use ooxml_base::Digest;
use ooxml_derive::{CacheBudget, Memo, MemoStats, Weigh};
use ooxml_word_layout::flow::FlowItem;
use ooxml_word_layout::paginate::{DrawnAs, LaidPage};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

/// What the cache holds at most by default, bytes: a 3,000-page document's two versions
/// in use (each page's lines' paths and digests) with room to spare.
pub const DIGEST_CACHE_BYTES: usize = 96 << 20;

/// A page about to be summarised: its laid page and the flow items it draws.
pub(crate) struct PageKey {
    pub(crate) page: Arc<LaidPage>,
    pub(crate) items: Vec<FlowItem>,
}

/// A page as laid out, held weakly: what it is drawn as ([`DrawnAs`]), by identity -- the
/// token its laid page and every page copied from it moved only in its flow's places hold.
#[derive(Clone)]
pub(crate) struct PageRef {
    drawn: Weak<DrawnAs>,
}

impl std::fmt::Debug for PageRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageRef").finish_non_exhaustive()
    }
}

impl PageRef {
    /// `key`'s page, held weakly.
    pub(crate) fn of(key: &PageKey) -> PageRef {
        PageRef {
            drawn: Arc::downgrade(&key.page.drawn_as),
        }
    }

    /// Whether it draws as `key`'s page does: drawn as the same laid page -- the very one,
    /// or one copied from it moved only in its flow's places, which draws the flow items it
    /// drew, moved with it ([`DrawnAs`]). The weak reference keeps the token's allocation, so
    /// its address is never another token's.
    pub(crate) fn drawn_as(&self, key: &PageKey) -> bool {
        std::ptr::eq(self.drawn.as_ptr(), Arc::as_ptr(&key.page.drawn_as))
    }

    /// Whether it draws as `other` does: drawn as the same laid page.
    pub(crate) fn drawn_alike(&self, other: &PageRef) -> bool {
        std::ptr::eq(self.drawn.as_ptr(), other.drawn.as_ptr())
    }

    /// Whether a version still holds a page drawn as it.
    pub(crate) fn alive(&self) -> bool {
        self.drawn.strong_count() > 0
    }

    /// What it holds besides itself: a weak reference holds its value's allocation (not
    /// what the value holds).
    pub(crate) fn weight(&self) -> usize {
        std::mem::size_of::<DrawnAs>()
    }
}

/// One page as digested.
#[derive(Clone)]
struct Kept {
    page: PageRef,
    /// The blocks the page draws, by key ([`crate::page_reads::page_blocks`]).
    blocks: PageBlocks,
    summary: Arc<PageSummary>,
}

impl Weigh for Kept {
    fn weight(&self) -> usize {
        let lines: usize = self
            .summary
            .lines
            .iter()
            .map(|l| {
                std::mem::size_of_val(l)
                    + l.path.as_ref().map_or(0, |path| path.len())
                    + l.id.as_ref().map_or(0, |id| id.len())
            })
            .sum();
        std::mem::size_of::<Kept>()
            + std::mem::size_of::<PageSummary>()
            + lines
            + self.blocks.weight()
            + self.page.weight()
    }
}

/// A page's key in the cache: its laid page by address, its context, and the stories'
/// epoch it was laid out in (what a part beside the main one changing moves on).
type CacheKey = (usize, Digest, u64);

/// The cache's own bounded account.
struct Bounded {
    used: AtomicUsize,
    limit: usize,
}

impl CacheBudget for Bounded {
    fn reserve(&self, bytes: usize) -> bool {
        self.used
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |used| {
                used.checked_add(bytes).filter(|total| *total <= self.limit)
            })
            .is_ok()
    }
    fn release(&self, bytes: usize) {
        // Never below zero: a release follows its reservation.
        let _ = self.used.try_update(Ordering::SeqCst, Ordering::SeqCst, |used| {
            Some(used.saturating_sub(bytes))
        });
    }
    fn used(&self) -> usize {
        self.used.load(Ordering::SeqCst)
    }
}

/// Each page's summary as digested, by the page as laid out and its context.
pub(crate) struct DigestCache {
    memo: Memo<CacheKey, Kept>,
}

impl std::fmt::Debug for DigestCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DigestCache")
            .field("stats", &self.memo.stats())
            .finish()
    }
}

impl DigestCache {
    /// An empty cache holding at most `limit` bytes.
    pub(crate) fn new(limit: usize) -> DigestCache {
        DigestCache {
            memo: Memo::new(Arc::new(Bounded {
                used: AtomicUsize::new(0),
                limit,
            })),
        }
    }

    /// The summary of the page `key`, drawing the blocks `blocks`, in `context` and `epoch`,
    /// where it was digested.
    pub(crate) fn get(
        &mut self,
        key: &PageKey,
        blocks: &PageBlocks,
        context: Digest,
        epoch: u64,
    ) -> Option<Arc<PageSummary>> {
        let found = self.memo.get(&(drawn(key), context, epoch))?;
        (found.page.drawn_as(key) && found.blocks == *blocks).then_some(found.summary)
    }

    /// Keep `summary` as the page `key`'s, drawing the blocks `blocks`, in `context` and
    /// `epoch`.
    pub(crate) fn insert(
        &mut self,
        key: &PageKey,
        blocks: PageBlocks,
        context: Digest,
        epoch: u64,
        summary: Arc<PageSummary>,
    ) {
        let kept = Kept {
            page: PageRef::of(key),
            blocks,
            summary,
        };
        self.memo.insert((drawn(key), context, epoch), kept);
    }

    /// Its counters: entries and bytes held among them.
    pub(crate) fn stats(&self) -> MemoStats {
        self.memo.stats()
    }
}

/// What the memo knows a page by: what it is drawn as (its allocation is kept while a kept
/// entry holds it weakly, so the address names no other).
fn drawn(key: &PageKey) -> usize {
    Arc::as_ptr(&key.page.drawn_as) as usize
}
