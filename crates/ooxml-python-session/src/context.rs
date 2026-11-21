//! Owns: `ooxml_core.CancelToken`, `ooxml_core.Limits` and `ooxml_core.CallContext` -- what a call runs with: who
//! calls (its key, and its context in the order given), a token that stops it, how long
//! laying a document out may take in it, how long it may run and how it writes (a `CallWriting`) -- and their conversion to the session's [`CallContext`].
//!
//! Must not: run a call, or touch a Python object without the lock.

use crate::options::PyCallWriting;
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

/// The host's limits on a call's reply (`ooxml_core.Limits`, the session's `Limits`):
/// `result_chars`, a page's characters (24,000 unless given), `list_items`, a listing's
/// items per page (50), `images_per_call`, the images one call gives at most (4), and
/// `image_edge`, an image's longest edge in pixels (2,576) -- docx-agent's
/// `Limits.max_result_chars`, `max_list_items`, `max_images_per_call` and `max_image_edge`.
/// One may serve many calls.
#[pyclass(frozen, name = "Limits", module = "ooxml_core")]
pub struct PyLimits {
    inner: ooxml_tools::Limits,
}

#[pymethods]
impl PyLimits {
    /// # Errors
    /// `ValueError` for a limit of 0.
    #[new]
    #[pyo3(signature = (*, result_chars = None, list_items = None, images_per_call = None, image_edge = None))]
    fn new(
        result_chars: Option<usize>,
        list_items: Option<usize>,
        images_per_call: Option<usize>,
        image_edge: Option<u32>,
    ) -> PyResult<Self> {
        let mut inner = ooxml_tools::Limits::DEFAULT;
        for (given, kept, name) in [
            (result_chars, &mut inner.result_chars, "result_chars"),
            (list_items, &mut inner.list_items, "list_items"),
            (images_per_call, &mut inner.images_per_call, "images_per_call"),
        ] {
            match given {
                Some(0) => return Err(PyValueError::new_err(format!("{name} is 1 or more"))),
                Some(n) => *kept = n,
                None => {}
            }
        }
        match image_edge {
            Some(0) => return Err(PyValueError::new_err("image_edge is 1 or more")),
            Some(edge) => inner.image_edge = i64::from(edge),
            None => {}
        }
        Ok(Self { inner })
    }

    /// A page's characters.
    #[getter]
    fn result_chars(&self) -> usize {
        self.inner.result_chars
    }

    /// A listing's items per page.
    #[getter]
    fn list_items(&self) -> usize {
        self.inner.list_items
    }

    /// The images one call gives at most.
    #[getter]
    fn images_per_call(&self) -> usize {
        self.inner.images_per_call
    }

    /// An image's longest edge, in pixels.
    #[getter]
    fn image_edge(&self) -> i64 {
        self.inner.image_edge
    }
}

/// What a call runs with (`ooxml_core.CallContext`): who calls (`caller`, its key; and
/// `context`, the caller's context as JSON in the order given, which replies name it by),
/// a token that stops it, how long it may run, in seconds, how long laying a document out
/// may take in it (`layout_timeout`, seconds), the `limits` on its reply (a `Limits`; the
/// defaults where none is given) and how it writes (`writing`, a `CallWriting`: its date, its
/// typography, the host's layout statistics a save writes and the locale of a table of
/// contents' style names; the session's clock, spaced, none and the session's locale unless
/// given). One may serve many calls.
#[pyclass(frozen, name = "CallContext", module = "ooxml_core")]
pub struct PyCallContext {
    pub(crate) inner: CallContext,
}

#[pymethods]
impl PyCallContext {
    /// # Errors
    /// `ValueError` for a negative deadline or layout timeout, or a context that is not a
    /// JSON object.
    #[new]
    #[pyo3(signature = (*, caller = None, cancel = None, deadline = None, layout_timeout = None,
                        limits = None, context = None, writing = None))]
    fn new(
        caller: Option<&str>,
        cancel: Option<PyRef<'_, PyCancelToken>>,
        deadline: Option<f64>,
        layout_timeout: Option<f64>,
        limits: Option<PyRef<'_, PyLimits>>,
        context: Option<&str>,
        writing: Option<PyRef<'_, PyCallWriting>>,
    ) -> PyResult<Self> {
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
                deadline: seconds(deadline, "deadline")?,
                layout_timeout: seconds(layout_timeout, "layout_timeout")?,
                limits: limits.map_or(ooxml_tools::Limits::DEFAULT, |l| l.inner),
                date: writing.as_ref().and_then(|w| w.date.clone()),
                counts: writing.as_ref().and_then(|w| w.counts),
                locale: writing.as_ref().and_then(|w| w.locale.clone()),
                as_given: writing.is_some_and(|w| w.as_given),
                ..CallContext::default()
            },
        })
    }
}

/// `value` seconds as a duration (`name` said where it is not 0 or more).
fn seconds(value: Option<f64>, name: &str) -> PyResult<Option<Duration>> {
    value
        .map(|seconds| {
            Duration::try_from_secs_f64(seconds)
                .map_err(|_| PyValueError::new_err(format!("{name} is seconds, 0 or more")))
        })
        .transpose()
}

/// The session's context of a call's `CallContext` (the default where none is given).
pub(crate) fn caller_context(context: Option<&PyCallContext>) -> CallContext {
    context.map(|c| c.inner.clone()).unwrap_or_default()
}
