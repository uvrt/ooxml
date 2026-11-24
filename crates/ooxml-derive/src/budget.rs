//! Owns: the cache-budget port a [`crate::Memo`] reserves its bytes from (design section 6,
//! Memory: caches belong to a document or a session and count against its budget).
//!
//! Also owns [`Limit`], a fixed bound for a cache kept where no session gives a budget.
//!
//! Must not: implement a session's budget; the session does. To be replaced by
//! `ooxml_base::CacheBudget` (lane P0a), which has these methods, once that crate is on
//! main.

/// A memory budget that caches reserve from and release to.
pub trait CacheBudget: Send + Sync {
    /// Reserve `bytes`; `false` when the budget cannot give them (the caller evicts or does
    /// without the cache).
    fn reserve(&self, bytes: usize) -> bool;
    /// Give back `bytes` reserved before.
    fn release(&self, bytes: usize);
    /// Bytes reserved now.
    fn used(&self) -> usize;
}

/// A budget of a fixed number of bytes: what a cache holds at most where no session's
/// account is given.
#[derive(Debug)]
pub struct Limit {
    limit: usize,
    used: std::sync::atomic::AtomicUsize,
}

impl Limit {
    /// A budget of `limit` bytes.
    #[must_use]
    pub fn new(limit: usize) -> Limit {
        Limit {
            limit,
            used: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl CacheBudget for Limit {
    fn reserve(&self, bytes: usize) -> bool {
        use std::sync::atomic::Ordering;
        self.used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|after| *after <= self.limit)
            })
            .is_ok()
    }

    fn release(&self, bytes: usize) {
        use std::sync::atomic::Ordering;
        let _ = self.used.try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
            Some(used.saturating_sub(bytes))
        });
    }

    fn used(&self) -> usize {
        self.used.load(std::sync::atomic::Ordering::Acquire)
    }
}
