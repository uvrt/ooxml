//! Owns: the clock port the store times its critical section with (design section 2.3).
//!
//! Must not: read the system clock. The adapter (the system's monotonic clock, a test
//! clock) is given by the caller. To be replaced by `ooxml_base::Clock` (lane P0a), whose
//! `monotonic` has this signature, once that crate is on main.

use std::time::Duration;

/// Monotonic time, for the store's timings and statistics.
pub trait Clock: Send + Sync {
    /// Time since an arbitrary fixed origin, never going backwards.
    fn monotonic(&self) -> Duration;
}
