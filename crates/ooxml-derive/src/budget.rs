//! Owns: the cache-budget port a [`crate::Memo`] reserves its bytes from (design section 6,
//! Memory: caches belong to a document or a session and count against its budget).
//!
//! Must not: implement a budget; the session does. To be replaced by
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
