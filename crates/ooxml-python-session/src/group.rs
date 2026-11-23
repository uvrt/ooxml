//! Owns: `ooxml_core.StepGroup` and `ooxml_core.GroupMark`, a document's step group as the
//! host's batch holds it (`Session.open_group`): marks, reverts to a mark and the close,
//! each made with the interpreter lock released.
//!
//! Must not: close a group itself -- the host's guard does, on every path.

use crate::context::{caller_context, PyCallContext};
use crate::session::to_py;
use crate::version::PyVersion;
use ooxml_session::{DocId, GroupMark, Session};
use pyo3::prelude::*;
use std::sync::Arc;

/// Where a step group was (`StepGroup.mark`), for `StepGroup.revert`.
#[pyclass(frozen, name = "GroupMark", module = "ooxml_core")]
pub struct PyGroupMark {
    inner: GroupMark,
}

/// A document's open step group (`Session.open_group`).
#[pyclass(frozen, name = "StepGroup", module = "ooxml_core")]
pub struct PyStepGroup {
    session: Arc<Session>,
    doc: DocId,
    start: GroupMark,
}

impl PyStepGroup {
    pub(crate) fn open(py: Python<'_>, session: Arc<Session>, doc: DocId) -> PyResult<Self> {
        let start = py.detach(|| session.open_group(&doc)).map_err(|e| to_py(&e))?;
        Ok(Self { session, doc, start })
    }

    fn head(&self, py: Python<'_>, made: Option<u64>) -> PyResult<Option<PyVersion>> {
        if made.is_none() {
            return Ok(None);
        }
        let opened = py.detach(|| self.session.snapshot(&self.doc)).map_err(|e| to_py(&e))?;
        Ok(Some(PyVersion { inner: opened }))
    }
}

#[pymethods]
impl PyStepGroup {
    /// Where the group is now.
    fn mark(&self, py: Python<'_>) -> PyResult<PyGroupMark> {
        let inner = py
            .detach(|| self.session.group_mark(&self.doc))
            .map_err(|e| to_py(&e))?;
        Ok(PyGroupMark { inner })
    }

    /// Take back what the group committed after `mark` (none given: since it opened): the
    /// new head (`Version`), or `None` when nothing was committed since.
    #[pyo3(signature = (mark = None, *, context = None))]
    fn revert(
        &self,
        py: Python<'_>,
        mark: Option<PyRef<'_, PyGroupMark>>,
        context: Option<PyRef<'_, PyCallContext>>,
    ) -> PyResult<Option<PyVersion>> {
        let mark = mark.map_or_else(|| self.start.clone(), |m| m.inner.clone());
        let context = caller_context(context.as_deref());
        let made = py
            .detach(|| self.session.revert_group(&self.doc, &mark, &context))
            .map_err(|e| to_py(&e))?;
        self.head(py, made)
    }

    /// The reflow of what the group committed, measured once (a batch's checks, after its
    /// last op; JSON text): from the version it started from to the head, laid out by
    /// `context`'s layout, else the document's own engine, where that engine laid the start
    /// out; `None` where nothing is measured. Each op's own reflow is not measured inside a
    /// group.
    #[pyo3(signature = (*, context = None))]
    fn reflow(&self, py: Python<'_>, context: Option<PyRef<'_, PyCallContext>>) -> PyResult<Option<String>> {
        let context = caller_context(context.as_deref());
        let found = py
            .detach(|| self.session.group_reflow(&self.doc, &context))
            .map_err(|e| to_py(&e))?;
        Ok(found.map(|r| r.to_string()))
    }

    /// Close the group: what it committed one step of the context's caller, made by `tool`
    /// (the batch's); the head (`Version`) it ends at, or `None` when it committed nothing.
    #[pyo3(signature = (*, context = None, tool = None))]
    fn close(
        &self,
        py: Python<'_>,
        context: Option<PyRef<'_, PyCallContext>>,
        tool: Option<&str>,
    ) -> PyResult<Option<PyVersion>> {
        let context = caller_context(context.as_deref());
        let made = py
            .detach(|| self.session.close_group(&self.doc, &context, tool))
            .map_err(|e| to_py(&e))?;
        self.head(py, made)
    }
}
