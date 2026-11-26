//! Owns: each page's summary as digested ([`DigestCache`]), kept across a generation's
//! versions by the page as laid out -- its `LaidPage` and the flow items it draws, by
//! identity -- and its context ([`crate::page_context`]): a page another version lays out
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
use ooxml_word_layout::paginate::LaidPage;
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

/// A flow item, held weakly.
#[derive(Clone)]
enum WeakItem {
    Para(Weak<ooxml_word_layout::flow::ParaItem>),
    Table(Weak<ooxml_word_layout::flow::TableItem>),
    Stop(Weak<ooxml_word_layout::flow::StopItem>),
}

impl WeakItem {
    fn of(item: &FlowItem) -> WeakItem {
        match item {
            FlowItem::Para(p) => WeakItem::Para(Arc::downgrade(p)),
            FlowItem::Table(t) => WeakItem::Table(Arc::downgrade(t)),
            FlowItem::Stop(s) => WeakItem::Stop(Arc::downgrade(s)),
        }
    }

    /// Whether it is `item` (a weak reference keeps its allocation: its address is never
    /// another item's).
    fn is(&self, item: &FlowItem) -> bool {
        match (self, item) {
            (WeakItem::Para(w), FlowItem::Para(a)) => std::ptr::eq(w.as_ptr(), Arc::as_ptr(a)),
            (WeakItem::Table(w), FlowItem::Table(a)) => std::ptr::eq(w.as_ptr(), Arc::as_ptr(a)),
            (WeakItem::Stop(w), FlowItem::Stop(a)) => std::ptr::eq(w.as_ptr(), Arc::as_ptr(a)),
            _ => false,
        }
    }

    /// Whether it is `other`.
    fn same(&self, other: &WeakItem) -> bool {
        match (self, other) {
            (WeakItem::Para(a), WeakItem::Para(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            (WeakItem::Table(a), WeakItem::Table(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            (WeakItem::Stop(a), WeakItem::Stop(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            _ => false,
        }
    }
}

/// A page as laid out, held weakly: its laid page and the flow items it draws, by
/// identity.
#[derive(Clone)]
pub(crate) struct PageRef {
    page: Weak<LaidPage>,
    items: Arc<[WeakItem]>,
}

impl std::fmt::Debug for PageRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageRef")
            .field("items", &self.items.len())
            .finish_non_exhaustive()
    }
}

impl PageRef {
    /// `key`'s page, held weakly.
    pub(crate) fn of(key: &PageKey) -> PageRef {
        PageRef {
            page: Arc::downgrade(&key.page),
            items: key.items.iter().map(WeakItem::of).collect(),
        }
    }

    /// Whether it is `key`'s page: the very laid page drawing the very items.
    pub(crate) fn is(&self, key: &PageKey) -> bool {
        std::ptr::eq(self.page.as_ptr(), Arc::as_ptr(&key.page))
            && self.items.len() == key.items.len()
            && self.items.iter().zip(&key.items).all(|(w, item)| w.is(item))
    }

    /// Whether it is the same page as `other`.
    pub(crate) fn same(&self, other: &PageRef) -> bool {
        std::ptr::eq(self.page.as_ptr(), other.page.as_ptr())
            && self.items.len() == other.items.len()
            && self.items.iter().zip(other.items.iter()).all(|(a, b)| a.same(b))
    }

    /// Whether a version still holds its laid page.
    pub(crate) fn alive(&self) -> bool {
        self.page.strong_count() > 0
    }

    /// What it holds besides itself: a weak reference holds its value's allocation (not
    /// what the value holds).
    pub(crate) fn weight(&self) -> usize {
        std::mem::size_of::<LaidPage>()
            + self.items.len()
                * (std::mem::size_of::<WeakItem>() + std::mem::size_of::<ooxml_word_layout::flow::ParaItem>())
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
        let found = self.memo.get(&(Arc::as_ptr(&key.page) as usize, context, epoch))?;
        (found.page.is(key) && found.blocks == *blocks).then_some(found.summary)
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
        self.memo
            .insert((Arc::as_ptr(&key.page) as usize, context, epoch), kept);
    }

    /// Its counters: entries and bytes held among them.
    pub(crate) fn stats(&self) -> MemoStats {
        self.memo.stats()
    }
}
