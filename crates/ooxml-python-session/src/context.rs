//! Owns: `ooxml_core.CancelToken` and `ooxml_core.CallContext` -- what a call runs with: who
//! calls (its key, and its context in the order given), a token that stops it, and how long
//! it may run -- and their conversion to the session's [`CallContext`].
//!
//! Must not: run a call, or touch a Python object without the lock.

use ooxml_session::{CallContext, CancelToken};
use ooxml_store::Caller;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// A token that stops the calls given it (`session.call(..., cancel=token)`): they stop at
/// their next safe point with `timeout` and change nothing.
#[pyclass(frozen, name = "CancelToken", module = "ooxml_core")]
pub struct PyCancelToken {
    inner: Arc<CancelToken>,
}

#[pymethods]
impl PyCancelToken {
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(CancelToken::new()),
        }
    }

    /// Stop the calls holding this token.
    fn cancel(&self) {
        self.inner.cancel();
    }

    /// Whether it was cancelled.
    #[getter]
    fn cancelled(&self) -> bool {
        ooxml_base::Cancel::is_cancelled(&*self.inner)
    }
}

/// What a call runs with (`ooxml_core.CallContext`): who calls (`caller`, its key; and
/// `context`, the caller's context as JSON in the order given, which replies name it by),
/// a token that stops it, and how long it may run, in seconds. One may serve many calls.
#[pyclass(frozen, name = "CallContext", module = "ooxml_core")]
pub struct PyCallContext {
    inner: CallContext,
}

#[pymethods]
impl PyCallContext {
    #[new]
    #[pyo3(signature = (*, caller = None, cancel = None, deadline = None, context = None))]
    fn new(
        caller: Option<&str>,
        cancel: Option<PyRef<'_, PyCancelToken>>,
        deadline: Option<f64>,
        context: Option<&str>,
    ) -> PyResult<Self> {
        let deadline = deadline
            .map(|seconds| {
                Duration::try_from_secs_f64(seconds)
                    .map_err(|_| PyValueError::new_err("deadline is seconds, 0 or more"))
            })
            .transpose()?;
        let caller_context = match context.map(serde_json::from_str::<Value>).transpose() {
            Ok(None) => None,
            Ok(Some(Value::Object(fields))) => Some(Arc::new(fields)),
            _ => return Err(PyValueError::new_err("context is a JSON object")),
        };
        Ok(Self {
            inner: CallContext {
                caller: caller.map_or_else(Caller::anonymous, Caller::named),
                caller_context,
                cancel: cancel.map(|t| Arc::clone(&t.inner)),
                deadline,
                ..CallContext::default()
            },
        })
    }
}

pub(crate) fn context_of(context: Option<&PyCallContext>) -> CallContext {
    context.map(|c| c.inner.clone()).unwrap_or_default()
}
