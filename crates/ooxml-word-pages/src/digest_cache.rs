//! Owns: each page's summary as digested ([`DigestCache`]), kept across a generation's
//! versions by the page as laid out -- its `LaidPage` and the flow items it draws, by
//! identity -- and its context ([`crate::page_context`]): a page another version lays out
//! the same, in the same context, is taken as digested, not painted again. It holds its
//! pages within its own bounded account, the least recently used let go first; it goes
//! with its generation.
//!
//! What it keeps of a page is never the page: only weak references tell it again (a page
//! no version holds any more is never found again, and its memory is not held).
//!
//! Must not: paint, keep a laid page or flow item alive, or grow past its bound.

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
}

/// One page as digested.
#[derive(Clone)]
struct Kept {
    page: Weak<LaidPage>,
    items: Arc<[WeakItem]>,
    summary: Arc<PageSummary>,
}

impl Weigh for Kept {
    fn weight(&self) -> usize {
        let lines: usize = self
            .summary
            .lines
            .iter()
            .map(|l| std::mem::size_of_val(l) + l.path.len() + l.id.as_ref().map_or(0, |id| id.len()))
            .sum();
        // A weak reference holds its value's allocation (not what the value holds).
        let held = std::mem::size_of::<LaidPage>()
            + self.items.len()
                * (std::mem::size_of::<WeakItem>() + std::mem::size_of::<ooxml_word_layout::flow::ParaItem>());
        std::mem::size_of::<Kept>() + std::mem::size_of::<PageSummary>() + lines + held
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

    /// The summary of the page `key` in `context` and `epoch`, where it was digested.
    pub(crate) fn get(&mut self, key: &PageKey, context: Digest, epoch: u64) -> Option<Arc<PageSummary>> {
        let found = self.memo.get(&(Arc::as_ptr(&key.page) as usize, context, epoch))?;
        let same = std::ptr::eq(found.page.as_ptr(), Arc::as_ptr(&key.page))
            && found.items.len() == key.items.len()
            && found.items.iter().zip(&key.items).all(|(w, item)| w.is(item));
        same.then_some(found.summary)
    }

    /// Keep `summary` as the page `key`'s in `context` and `epoch`.
    pub(crate) fn insert(&mut self, key: &PageKey, context: Digest, epoch: u64, summary: Arc<PageSummary>) {
        let kept = Kept {
            page: Arc::downgrade(&key.page),
            items: key.items.iter().map(WeakItem::of).collect(),
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
