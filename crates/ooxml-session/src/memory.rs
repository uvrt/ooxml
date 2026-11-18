//! Owns: a session's [`MemoryBudget`] (docs/architecture/README.md, section 6, Memory):
//! what the session's documents may keep of what can always be made again -- page paint,
//! the layouts kept for readers -- within one limit, each document's share counted on its
//! own [`Account`]; and what they hold that they need (the memos of the version in use),
//! counted beside it, never refused.
//!
//! Must not: evict anything itself (each cache evicts its own least recently used entries
//! when a reservation is refused, `ooxml_derive::Memo`), read the environment or the
//! machine (the host gives the limit), or rely on an interpreter lock: every count is
//! atomic.

use ooxml_derive::CacheBudget;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A session's memory budget.
#[derive(Debug)]
pub struct MemoryBudget {
    limit: usize,
    /// Bytes reserved within the limit, every account's together.
    used: AtomicUsize,
    /// Bytes held beside it (the memos of the versions in use), every account's together.
    held: AtomicUsize,
}

impl MemoryBudget {
    /// A budget of `limit` bytes for what the session's documents keep that can be made
    /// again.
    pub fn new(limit: usize) -> Arc<MemoryBudget> {
        Arc::new(MemoryBudget {
            limit,
            used: AtomicUsize::new(0),
            held: AtomicUsize::new(0),
        })
    }

    /// The limit, bytes.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Bytes reserved within the limit now.
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }

    /// Bytes held beside the limit now: what the versions in use need.
    pub fn held(&self) -> usize {
        self.held.load(Ordering::Acquire)
    }

    /// A new account (one document's) of what can be made again: its reservations are
    /// granted while the session's total stays within the limit.
    pub fn account(self: &Arc<Self>) -> Arc<Account> {
        Arc::new(Account {
            session: Arc::clone(self),
            used: AtomicUsize::new(0),
            bounded: true,
        })
    }

    /// A new account (one document's) of what it needs: every reservation is granted and
    /// counted as held.
    pub fn holding(self: &Arc<Self>) -> Arc<Account> {
        Arc::new(Account {
            session: Arc::clone(self),
            used: AtomicUsize::new(0),
            bounded: false,
        })
    }

    fn total(&self, bounded: bool) -> &AtomicUsize {
        if bounded {
            &self.used
        } else {
            &self.held
        }
    }
}

/// One document's account in a session's [`MemoryBudget`].
#[derive(Debug)]
pub struct Account {
    session: Arc<MemoryBudget>,
    used: AtomicUsize,
    bounded: bool,
}

impl Account {
    /// Whether its reservations count against the session's limit (else they are held).
    pub fn bounded(&self) -> bool {
        self.bounded
    }
}

impl CacheBudget for Account {
    fn reserve(&self, bytes: usize) -> bool {
        let total = self.session.total(self.bounded);
        if self.bounded {
            let limit = self.session.limit;
            let mut now = total.load(Ordering::Acquire);
            loop {
                let Some(after) = now.checked_add(bytes).filter(|after| *after <= limit) else {
                    return false;
                };
                match total.compare_exchange_weak(now, after, Ordering::AcqRel, Ordering::Acquire) {
                    Ok(_) => break,
                    Err(seen) => now = seen,
                }
            }
        } else {
            total.fetch_add(bytes, Ordering::AcqRel);
        }
        self.used.fetch_add(bytes, Ordering::AcqRel);
        true
    }

    fn release(&self, bytes: usize) {
        let bytes = bytes.min(self.used.load(Ordering::Acquire));
        self.used.fetch_sub(bytes, Ordering::AcqRel);
        self.session.total(self.bounded).fetch_sub(bytes, Ordering::AcqRel);
    }

    fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
}

impl Drop for Account {
    fn drop(&mut self) {
        // What a document's caches did not give back when it went (they give it back as
        // they drop; this is what remains), so the session's total stays true.
        let left = *self.used.get_mut();
        self.session.total(self.bounded).fetch_sub(left, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_share_the_limit_and_holding_is_never_refused() {
        let budget = MemoryBudget::new(100);
        let (a, b, h) = (budget.account(), budget.account(), budget.holding());
        assert!(a.reserve(60));
        assert!(!b.reserve(50));
        assert!(b.reserve(40));
        assert!(h.reserve(1_000));
        assert_eq!((budget.used(), budget.held()), (100, 1_000));
        a.release(60);
        assert_eq!(budget.used(), 40);
        drop(b);
        assert_eq!(budget.used(), 0);
        assert_eq!((a.used(), h.used()), (0, 1_000));
    }
}
