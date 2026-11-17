//! Owns: the store's counters (design section 10: commits, conflicts, redos, rebases, exclusive
//! runs, the critical section's time) as atomics on the store, read with
//! [`crate::Store::stats`].
//!
//! Must not: be process-wide; every store has its own.

use crate::sync::{AtomicU64, Ordering};
use std::time::Duration;

/// A copy of a store's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StoreStats {
    /// Transactions committed with a change (undo and redo included).
    pub commits: u64,
    /// Commits refused for a conflict and prepared again.
    pub conflicts: u64,
    /// Commits whose changes were rebased past others committed since their snapshot.
    pub rebased: u64,
    /// Transactions run under the commit lock after too many conflicts.
    pub exclusive: u64,
    /// Time inside the commit lock, summed over commits.
    pub held_total: Duration,
    /// The longest time inside the commit lock.
    pub held_max: Duration,
}

pub(crate) struct Counts {
    commits: AtomicU64,
    conflicts: AtomicU64,
    rebased: AtomicU64,
    exclusive: AtomicU64,
    held_ns: AtomicU64,
    held_max_ns: AtomicU64,
}

impl Counts {
    pub(crate) fn new() -> Self {
        Self {
            commits: AtomicU64::new(0),
            conflicts: AtomicU64::new(0),
            rebased: AtomicU64::new(0),
            exclusive: AtomicU64::new(0),
            held_ns: AtomicU64::new(0),
            held_max_ns: AtomicU64::new(0),
        }
    }

    pub(crate) fn committed(&self, held: Duration) {
        let ns = u64::try_from(held.as_nanos()).unwrap_or(u64::MAX);
        self.commits.fetch_add(1, Ordering::Relaxed);
        self.held_ns.fetch_add(ns, Ordering::Relaxed);
        self.held_max_ns.fetch_max(ns, Ordering::Relaxed);
    }

    pub(crate) fn conflict(&self) {
        self.conflicts.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn rebased(&self) {
        self.rebased.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn exclusive(&self) {
        self.exclusive.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn read(&self) -> StoreStats {
        StoreStats {
            commits: self.commits.load(Ordering::Relaxed),
            conflicts: self.conflicts.load(Ordering::Relaxed),
            rebased: self.rebased.load(Ordering::Relaxed),
            exclusive: self.exclusive.load(Ordering::Relaxed),
            held_total: Duration::from_nanos(self.held_ns.load(Ordering::Relaxed)),
            held_max: Duration::from_nanos(self.held_max_ns.load(Ordering::Relaxed)),
        }
    }
}
