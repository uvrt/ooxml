//! Owns: a version's layout summary as Python holds it (`ooxml_core.LayoutSummary`, lane
//! L4b): made by the version's engine, compared with another version's in Rust -- the
//! changed pages, why, the blocks moved -- ids followed through the aliases the caller
//! hands over. docx-agent's `LayoutSummary` wraps it; nothing of it is a Python object
//! but what a comparison answers.
//!
//! Must not: lay out, paint or keep a Python object.

use ooxml_word_pages::summary::Summary;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

/// A line of a page as Python reads it: `(id, path, body, digest)`.
type SummaryRow<'py> = (Option<String>, String, bool, Bound<'py, PyBytes>);
use std::collections::HashMap;
use std::sync::Arc;

/// One version's layout summary.
#[pyclass(frozen, name = "LayoutSummary", module = "ooxml_core")]
pub struct PyLayoutSummary {
    pub(crate) inner: Arc<Summary>,
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
        self.inner.known
    }

    /// Every page as `(rest, [(id, path, body, digest)])`, digests as bytes: what a
    /// comparison with a summary made in Python reads.
    fn pages<'py>(&self, py: Python<'py>) -> Vec<(Bound<'py, PyBytes>, Vec<SummaryRow<'py>>)> {
        self.inner
            .pages
            .iter()
            .map(|page| {
                let lines = page
                    .lines
                    .iter()
                    .map(|l| {
                        (
                            l.id.as_deref().map(str::to_owned),
                            l.path.to_string(),
                            l.body,
                            PyBytes::new(py, &l.drawn.0),
                        )
                    })
                    .collect();
                (PyBytes::new(py, &page.rest.0), lines)
            })
            .collect()
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
        let (now, then) = (Arc::clone(&self.inner), Arc::clone(&before.inner));
        let reflow = py.detach(|| now.compare(&then, &aliases));
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
