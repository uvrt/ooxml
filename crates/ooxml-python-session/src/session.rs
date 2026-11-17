//! Owns: `ooxml_core.Session`, `ooxml_core.CancelToken` and `ooxml_core.tool_specs`: their
//! Python signatures, the copying of Python values into Rust ones while the interpreter
//! lock is held, every session call made with it released, and the mapping of errors.
//!
//! Must not: do a session's work itself, or touch a Python object without the lock.

use crate::adapters::{context, SessionStop};
use ooxml_base::Locale;
use ooxml_session::{CallContext, CancelToken, DocId, DocKind, Registry, Session, SessionConfig, SessionError};
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::{compact, ToolError, ToolResult};
use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyTimeoutError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use serde_json::Value;
use std::panic::{catch_unwind, AssertUnwindSafe};
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

/// Documents by id, their versions, and the tools that read them. Every method releases
/// the interpreter lock for its Rust work; any number of threads may use one session.
#[pyclass(frozen, name = "Session", module = "ooxml_core")]
pub struct PySession {
    inner: Arc<Session>,
    stop: Arc<SessionStop>,
}

fn kind_of(kind: &str) -> PyResult<DocKind> {
    DocKind::parse(kind).ok_or_else(|| to_py(&SessionError::Kind(kind.to_owned())))
}

fn doc_id(doc: &str, session: &Session) -> PyResult<DocId> {
    DocId::parse(doc).ok_or_else(|| {
        to_py(&SessionError::NotOpen {
            id: doc.to_owned(),
            open: session.documents().iter().map(ToString::to_string).collect(),
        })
    })
}

fn to_py(error: &SessionError) -> PyErr {
    let message = error.to_string();
    match error {
        SessionError::NotOpen { .. } => PyKeyError::new_err(message),
        SessionError::Kind(_) | SessionError::PartName(_) | SessionError::Package(_) => PyValueError::new_err(message),
        SessionError::Base(ooxml_base::BaseError::Cancelled) => PyTimeoutError::new_err(message),
        SessionError::Store(_) | SessionError::Base(_) => PyRuntimeError::new_err(message),
    }
}

/// What a call runs with (`ooxml_core.CallContext`): who calls, a token that stops it,
/// and how long it may run, in seconds. One may serve many calls.
#[pyclass(frozen, name = "CallContext", module = "ooxml_core")]
pub struct PyCallContext {
    inner: CallContext,
}

#[pymethods]
impl PyCallContext {
    #[new]
    #[pyo3(signature = (*, caller = None, cancel = None, deadline = None))]
    fn new(caller: Option<&str>, cancel: Option<PyRef<'_, PyCancelToken>>, deadline: Option<f64>) -> PyResult<Self> {
        let deadline = deadline
            .map(|seconds| {
                Duration::try_from_secs_f64(seconds)
                    .map_err(|_| PyValueError::new_err("deadline is seconds, 0 or more"))
            })
            .transpose()?;
        Ok(Self {
            inner: CallContext {
                caller: caller.map_or_else(Caller::anonymous, Caller::named),
                cancel: cancel.map(|t| Arc::clone(&t.inner)),
                deadline,
            },
        })
    }
}

fn context_of(context: Option<&PyCallContext>) -> CallContext {
    context.map(|c| c.inner.clone()).unwrap_or_default()
}

#[pymethods]
impl PySession {
    /// A session: each document keeps `history_depth` undo steps; `locale` is the BCP 47
    /// tag the session's documents are read under.
    #[new]
    #[pyo3(signature = (*, history_depth = 50, locale = "en-US"))]
    fn new(history_depth: usize, locale: &str) -> PyResult<Self> {
        let locale = Locale::new(locale).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let stop = Arc::new(SessionStop::default());
        let config = SessionConfig {
            store: StoreConfig {
                history_depth,
                ..StoreConfig::default()
            },
        };
        let inner = Session::new(config, context(Arc::clone(&stop), locale));
        Ok(Self {
            inner: Arc::new(inner),
            stop,
        })
    }

    /// Open a package's bytes as a document of `kind`, named `name`; returns its id (`d1`).
    #[pyo3(signature = (data, *, kind = "docx", name = "document.docx"))]
    fn open(&self, py: Python<'_>, data: &[u8], kind: &str, name: &str) -> PyResult<String> {
        let kind = kind_of(kind)?;
        let bytes: Arc<[u8]> = Arc::from(data);
        let session = &self.inner;
        let id = py.detach(|| session.open(bytes, kind, name)).map_err(|e| to_py(&e))?;
        Ok(id.to_string())
    }

    /// Run the tool `name` with `arguments` (JSON text) on a snapshot, under `context`: returns the reply as
    /// JSON text -- the whole envelope, or within `limit` characters as the model is sent
    /// it -- and the images beside it (bytes). Never raises for the call's own failure:
    /// that is the reply's `error`, with its way out.
    #[pyo3(signature = (name, arguments, *, context = None, limit = None))]
    fn call(
        &self,
        py: Python<'_>,
        name: &str,
        arguments: String,
        context: Option<PyRef<'_, PyCallContext>>,
        limit: Option<usize>,
    ) -> (String, Vec<Py<PyBytes>>) {
        let context = context_of(context.as_deref());
        let session = &self.inner;
        let text = py.detach(|| {
            let run = || session.call(name, Value::String(arguments), &context);
            let result = catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|panic| {
                let what = panic
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_owned())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_default();
                ToolResult::failed(ToolError::internal(
                    format!("the call panicked: {what}"),
                    &format!("call:{name}"),
                ))
            });
            match limit {
                Some(limit) => result.to_text(Some(limit)),
                None => compact(&Value::Object(result.to_json())),
            }
        });
        (text, Vec::new())
    }

    /// Commit a version's touched parts (bytes, or `None` to remove the part) as one step:
    /// returns `(version, changed, attempts)`.
    #[pyo3(signature = (doc, parts, *, context = None))]
    fn update(
        &self,
        py: Python<'_>,
        doc: &str,
        parts: &Bound<'_, PyDict>,
        context: Option<PyRef<'_, PyCallContext>>,
    ) -> PyResult<(u64, bool, u32)> {
        let id = doc_id(doc, &self.inner)?;
        let context = context_of(context.as_deref());
        let mut copied = Vec::with_capacity(parts.len());
        for (name, data) in parts.iter() {
            let name: String = name.extract()?;
            let data: Option<Vec<u8>> = data.extract()?;
            copied.push((name, data.map(Arc::<[u8]>::from)));
        }
        let session = &self.inner;
        let updated = py
            .detach(|| session.update(&id, copied, &context))
            .map_err(|e| to_py(&e))?;
        Ok((updated.version, updated.changed, updated.attempts))
    }

    /// The document's bytes at its head version.
    fn save<'py>(&self, py: Python<'py>, doc: &str) -> PyResult<Bound<'py, PyBytes>> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        let bytes = py.detach(|| session.save(&id)).map_err(|e| to_py(&e))?;
        Ok(PyBytes::new(py, &bytes))
    }

    /// Close the document; its versions and history are let go.
    fn close(&self, py: Python<'_>, doc: &str) -> PyResult<()> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        py.detach(|| session.close(&id)).map_err(|e| to_py(&e))
    }

    /// The document's head version number. Never waits for a writer.
    fn version(&self, py: Python<'_>, doc: &str) -> PyResult<u64> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        py.detach(|| session.snapshot(&id).map(|s| s.version()))
            .map_err(|e| to_py(&e))
    }

    /// The open documents' ids, in the order they were opened.
    fn documents(&self, py: Python<'_>) -> Vec<String> {
        let session = &self.inner;
        py.detach(|| session.documents().iter().map(ToString::to_string).collect())
    }

    /// The tools' definitions for `kind` as JSON text: a list of `ToolSpec` exports, which
    /// the Python adapters turn into `Tool`s.
    fn tool_specs(&self, py: Python<'_>, kind: &str) -> PyResult<String> {
        let kind = kind_of(kind)?;
        let session = &self.inner;
        Ok(py.detach(|| compact(&Value::Array(session.registry().export(kind)))))
    }

    /// The session's counters: calls, failed, cancelled, updates, open.
    fn stats(&self, py: Python<'_>) -> Vec<(&'static str, u64)> {
        let s = py.detach(|| self.inner.stats());
        vec![
            ("calls", s.calls),
            ("failed", s.failed),
            ("cancelled", s.cancelled),
            ("updates", s.updates),
            ("open", s.open),
        ]
    }

    /// The document's store counters: commits, conflicts, exclusive runs.
    fn store_stats(&self, py: Python<'_>, doc: &str) -> PyResult<Vec<(&'static str, u64)>> {
        let id = doc_id(doc, &self.inner)?;
        let s = py.detach(|| self.inner.store_stats(&id)).map_err(|e| to_py(&e))?;
        Ok(vec![
            ("commits", s.commits),
            ("conflicts", s.conflicts),
            ("exclusive", s.exclusive),
        ])
    }

    /// Stop every call in flight at its next safe point (the session stays usable for
    /// nothing after: each later call stops at once).
    fn shutdown(&self) {
        self.stop.stop();
    }
}

/// The tools' definitions for `kind` as JSON text (a list of `ToolSpec` exports): what a
/// session made now would serve.
#[pyfunction]
pub fn tool_specs(py: Python<'_>, kind: &str) -> PyResult<String> {
    let kind = kind_of(kind)?;
    Ok(py.detach(|| compact(&Value::Array(Registry::standard().export(kind)))))
}
