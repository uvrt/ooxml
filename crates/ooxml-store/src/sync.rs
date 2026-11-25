//! Owns: the synchronisation primitives the commit path is built from, so that loom can
//! model-check that path (tests/loom.rs) with its own versions of them.
//!
//! Must not: hold logic. Under `cfg(loom)` the head cell is a loom mutex around the pointer,
//! because arc-swap's internals are not loom-aware; every caller above this module is the
//! same code in both builds. Ordinary builds never compile the loom side.

use std::sync::Arc;

#[cfg(loom)]
pub(crate) use loom::sync::atomic::{AtomicU64, Ordering};
#[cfg(loom)]
pub(crate) use loom::sync::{Condvar, Mutex, MutexGuard};
#[cfg(not(loom))]
pub(crate) use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(loom))]
pub(crate) use std::sync::{Condvar, Mutex, MutexGuard};

/// Lock `mutex`, taking the state as it is when an earlier holder panicked: every critical
/// section in this crate computes all that can fail before it changes anything, so a panic
/// leaves the state as it was.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Wait on `condvar` with `guard`'s lock let go meanwhile (a panic elsewhere ignored, as
/// [`lock`]).
pub(crate) fn wait<'m, T>(condvar: &Condvar, guard: MutexGuard<'m, T>) -> MutexGuard<'m, T> {
    condvar.wait(guard).unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The published head: loads never wait for a writer.
#[cfg(not(loom))]
pub(crate) struct HeadCell<T>(arc_swap::ArcSwap<T>);

#[cfg(not(loom))]
impl<T> HeadCell<T> {
    pub(crate) fn new(value: Arc<T>) -> Self {
        Self(arc_swap::ArcSwap::new(value))
    }
    pub(crate) fn load(&self) -> Arc<T> {
        self.0.load_full()
    }
    pub(crate) fn store(&self, value: Arc<T>) {
        self.0.store(value);
    }
}

/// The published head, as loom models it.
#[cfg(loom)]
pub(crate) struct HeadCell<T>(Mutex<Arc<T>>);

#[cfg(loom)]
impl<T> HeadCell<T> {
    pub(crate) fn new(value: Arc<T>) -> Self {
        Self(Mutex::new(value))
    }
    pub(crate) fn load(&self) -> Arc<T> {
        Arc::clone(&lock(&self.0))
    }
    pub(crate) fn store(&self, value: Arc<T>) {
        *lock(&self.0) = value;
    }
}
