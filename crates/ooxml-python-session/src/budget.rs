//! Owns: the session's memory budget as Python holds it (`ooxml_core.MemoryBudget` and
//! `ooxml_core.BudgetAccount`, lane L4b): its limit, what it holds, and accounts a host's
//! own caches reserve from.
//!
//! Must not: evict anything (each cache evicts its own), or keep a Python object.

use ooxml_derive::CacheBudget;
use ooxml_session::{Account, MemoryBudget};
use pyo3::prelude::*;
use std::sync::Arc;

/// What a budget made for an engine alone holds when none is given: its pages' paint.
pub(crate) const OWN_BUDGET: usize = 64 * 1024 * 1024;

/// A session's memory budget (`MemoryBudget`): what its documents keep that can be made
/// again -- page paint, layouts kept for readers -- within `limit` bytes, each document
/// counted apart; and, beside it, what their versions in use hold (`held`).
#[pyclass(frozen, name = "MemoryBudget", module = "ooxml_core")]
pub struct PyMemoryBudget {
    pub(crate) inner: Arc<MemoryBudget>,
}

#[pymethods]
impl PyMemoryBudget {
    /// A budget of `limit` bytes.
    #[new]
    fn new(limit: usize) -> Self {
        Self {
            inner: MemoryBudget::new(limit),
        }
    }

    /// The limit, bytes.
    #[getter]
    fn limit(&self) -> usize {
        self.inner.limit()
    }

    /// Bytes reserved within the limit now.
    #[getter]
    fn used(&self) -> usize {
        self.inner.used()
    }

    /// Bytes the engines' versions in use hold now (their memos), beside the limit.
    #[getter]
    fn held(&self) -> usize {
        self.inner.held()
    }

    /// A new account of this budget, for a host's own cache (a document's kept layouts):
    /// `reserve(n)` is refused past the limit, `release(n)` gives back.
    fn account(&self) -> PyBudgetAccount {
        PyBudgetAccount {
            inner: self.inner.account(),
        }
    }
}

/// One cache's account in a `MemoryBudget`.
#[pyclass(frozen, name = "BudgetAccount", module = "ooxml_core")]
pub struct PyBudgetAccount {
    inner: Arc<Account>,
}

#[pymethods]
impl PyBudgetAccount {
    /// Reserve `bytes`: false where the budget has no room (the caller lets something go,
    /// or keeps nothing).
    fn reserve(&self, bytes: usize) -> bool {
        self.inner.reserve(bytes)
    }

    /// Give back `bytes` reserved before.
    fn release(&self, bytes: usize) {
        self.inner.release(bytes);
    }

    /// Bytes this account holds now.
    #[getter]
    fn used(&self) -> usize {
        self.inner.used()
    }
}
