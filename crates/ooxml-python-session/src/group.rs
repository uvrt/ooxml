//! Owns: `ooxml_core.StepGroup` and `ooxml_core.GroupMark`, a document's step group as the
//! host's batch holds it (`Session.open_group`): marks, reverts to a mark and the close,
//! each made with the interpreter lock released.
//!
//! Must not: close a group itself -- the host's guard does, on every path.

use crate::session::{context_of, to_py, PyCallContext, PyVersion};
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
        let context = context_of(context.as_deref());
        let made = py
            .detach(|| self.session.revert_group(&self.doc, &mark, &context))
            .map_err(|e| to_py(&e))?;
        self.head(py, made)
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
        let context = context_of(context.as_deref());
        let made = py
            .detach(|| self.session.close_group(&self.doc, &context, tool))
            .map_err(|e| to_py(&e))?;
        self.head(py, made)
    }
}
