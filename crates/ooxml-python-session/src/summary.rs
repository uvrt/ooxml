//! Owns: a version's layout summary as Python holds it (`ooxml_core.LayoutSummary`, lane
//! L4b): made by the version's engine, compared with another version's in Rust -- the
//! changed pages, why, the blocks moved -- ids followed through the aliases the caller
//! hands over. docx-agent's `LayoutSummary` wraps it; nothing of it is a Python object
//! but what a comparison answers. What the session's last check saw is held as the
//! version laid out (lane P5e): summarised only when Python reads its pages or compares
//! it (two versions of one engine page by page).
//!
//! Must not: lay out, paint or keep a Python object.

use ooxml_word_pages::summarised::Summarised;
use ooxml_word_pages::summary::Summary;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

/// A line of a page as Python reads it: `(id, path, body, digest)`.
type SummaryRow<'py> = (Option<String>, String, bool, Bound<'py, PyBytes>);
use std::collections::HashMap;
use std::sync::Arc;

/// One version's layout summary.
#[pyclass(frozen, name = "LayoutSummary", module = "ooxml_core")]
pub struct PyLayoutSummary {
    pub(crate) inner: Summarised,
}

/// What a summary that can no longer be made raises.
fn gone() -> PyErr {
    PyRuntimeError::new_err("a version its engine no longer lays out")
}

impl PyLayoutSummary {
    /// Its summary whole, made where it was not (its engine painting what it needs).
    fn whole(&self, py: Python<'_>) -> PyResult<Arc<Summary>> {
        let inner = self.inner.clone();
        py.detach(|| inner.summary()).ok_or_else(gone)
    }
}

#[pymethods]
impl PyLayoutSummary {
    /// Its pages.
    #[getter]
    fn count(&self) -> usize {
        self.inner.count()
    }

    /// Its pages known.
    #[getter]
    fn known(&self) -> usize {
        self.inner.known()
    }

    /// Every page as `(rest, [(id, path, body, digest)])`, digests as bytes: what a
    /// comparison with a summary made in Python reads.
    fn pages<'py>(&self, py: Python<'py>) -> PyResult<Vec<(Bound<'py, PyBytes>, Vec<SummaryRow<'py>>)>> {
        let whole = self.whole(py)?;
        Ok(whole
            .pages
            .iter()
            .map(|page| {
                let lines = page
                    .lines
                    .iter()
                    .map(|l| {
                        (
                            l.id.as_deref().map(str::to_owned),
                            whole.path_of(l).unwrap_or_default(),
                            l.body,
                            PyBytes::new(py, &l.drawn.0),
                        )
                    })
                    .collect();
                (PyBytes::new(py, &page.rest.0), lines)
            })
            .collect())
    }

    /// What changed from `before` to this summary, ids followed through `aliases` (id ->
    /// the id it became): `{"changed", "page_count", "moved", "unknown", "why"}` --
    /// `moved` a list of `(id, before, after)` in the order this version first draws them.
    fn compare<'py>(
        &self,
        py: Python<'py>,
        before: PyRef<'_, PyLayoutSummary>,
        aliases: HashMap<String, String>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let (now, then) = (self.inner.clone(), before.inner.clone());
        let reflow = py.detach(|| now.compare(&then, &aliases)).ok_or_else(gone)?;
        let out = PyDict::new(py);
        out.set_item("changed", reflow.changed)?;
        out.set_item("page_count", reflow.page_count)?;
        let moved: Vec<(String, usize, usize)> = reflow.moved.into_iter().map(|(id, (a, b))| (id, a, b)).collect();
        out.set_item("moved", moved)?;
        out.set_item("unknown", reflow.unknown)?;
        let why: Vec<(usize, &str)> = reflow.why.into_iter().collect();
        out.set_item("why", why)?;
        Ok(out)
    }
}
