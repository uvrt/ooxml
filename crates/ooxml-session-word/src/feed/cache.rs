//! Owns: the changes feed's SVG of each page, latest only ([`SvgCache`]): each page as the
//! feed wrote it, with the seq it was written at, reserved from the document's memory
//! budget and let go least recently sent first when the budget has no room (design section
//! 4, its second place in the eviction order); a page let go is written again on demand.
//!
//! Must not: be the source of truth (the records are), paint, or hold its lock while
//! anything but its map is read or changed.

use ooxml_base::CacheBudget;
use ooxml_word_layout::paint::feed::FeedPage;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// One page kept.
struct Kept {
    /// The seq it was written at.
    at: u64,
    page: Arc<FeedPage>,
    bytes: usize,
    /// When it was last sent (the cache's own clock).
    sent: u64,
}

#[derive(Default)]
struct Pages {
    kept: HashMap<usize, Kept>,
    clock: u64,
}

/// The pages' SVG, latest only, within the budget.
pub(crate) struct SvgCache {
    budget: Arc<dyn CacheBudget>,
    pages: Mutex<Pages>,
}

/// What a page's SVG weighs on the budget.
fn weight(page: &FeedPage) -> usize {
    page.groups
        .iter()
        .map(|g| g.id.len() + g.content.len() + g.defs.len() + 64)
        .sum::<usize>()
        + 128
}

impl SvgCache {
    /// None kept; reserved from `budget`.
    pub(crate) fn new(budget: Arc<dyn CacheBudget>) -> SvgCache {
        SvgCache {
            budget,
            pages: Mutex::new(Pages::default()),
        }
    }

    fn pages(&self) -> MutexGuard<'_, Pages> {
        self.pages.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Page `page` as written at a seq in `valid` (from the seq it last changed at to the
    /// records' seq), made the most recently sent.
    pub(crate) fn get(&self, page: usize, valid: std::ops::RangeInclusive<u64>) -> Option<Arc<FeedPage>> {
        let mut pages = self.pages();
        pages.clock += 1;
        let clock = pages.clock;
        let found = pages.kept.get_mut(&page).filter(|k| valid.contains(&k.at))?;
        found.sent = clock;
        Some(Arc::clone(&found.page))
    }

    /// Keep `made` as page `page` written at `at`, letting the least recently sent others go
    /// while the budget has no room (and `room` -- the next place in the eviction order --
    /// frees none); not kept where it still has none.
    pub(crate) fn keep(&self, page: usize, at: u64, made: &Arc<FeedPage>, room: &dyn Fn() -> bool) {
        let bytes = weight(made);
        let mut pages = self.pages();
        if let Some(old) = pages.kept.remove(&page) {
            self.budget.release(old.bytes);
        }
        loop {
            if self.budget.reserve(bytes) {
                break;
            }
            if room() {
                continue;
            }
            if !self.let_go_one(&mut pages) {
                return;
            }
        }
        pages.clock += 1;
        let sent = pages.clock;
        pages.kept.insert(
            page,
            Kept {
                at,
                page: Arc::clone(made),
                bytes,
                sent,
            },
        );
    }

    /// Let the least recently sent page go; `false` where none is kept.
    fn let_go_one(&self, pages: &mut Pages) -> bool {
        let Some(page) = pages.kept.iter().min_by_key(|(_, k)| k.sent).map(|(p, _)| *p) else {
            return false;
        };
        if let Some(gone) = pages.kept.remove(&page) {
            self.budget.release(gone.bytes);
        }
        true
    }

    /// Let the least recently sent page go, for another's room; `false` where none is kept.
    pub(crate) fn evict_one(&self) -> bool {
        let mut pages = self.pages();
        self.let_go_one(&mut pages)
    }

    /// Let go every page past the last of `count`.
    pub(crate) fn truncate(&self, count: usize) {
        let mut pages = self.pages();
        let gone: Vec<usize> = pages.kept.keys().copied().filter(|p| *p >= count).collect();
        for page in gone {
            if let Some(k) = pages.kept.remove(&page) {
                self.budget.release(k.bytes);
            }
        }
    }
}

impl Drop for SvgCache {
    fn drop(&mut self) {
        let pages = self.pages.get_mut().unwrap_or_else(PoisonError::into_inner);
        for (_, k) in pages.kept.drain() {
            self.budget.release(k.bytes);
        }
    }
}
