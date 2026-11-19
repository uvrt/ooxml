//! Owns: `ooxml_core.Session`, `ooxml_core.CancelToken`, `ooxml_core.CallContext`,
//! `ooxml_core.Version` (a document's version a call may be pinned to) and
//! `ooxml_core.tool_specs`: their Python signatures, the adapter of a Python placer (a
//! `Placer` called with the interpreter lock taken back), the copying of Python values into Rust ones while the interpreter
//! lock is held, every session call made with it released, and the mapping of errors.
//!
//! Must not: do a session's work itself, or touch a Python object without the lock.

use crate::adapters::{context, SessionStop};
use ooxml_base::Locale;
use ooxml_session::{CallContext, CancelToken, DocId, DocKind, Opened, Registry, Session, SessionConfig, SessionError};
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::{compact, ToolError, ToolResult};
use ooxml_tools_word::Placer;
use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyTimeoutError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use serde_json::Map;
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
        SessionError::Word(_) => PyValueError::new_err(message),
        SessionError::Entangled { .. } | SessionError::NewProblems { .. } => PyValueError::new_err(message),
        SessionError::Store(_) | SessionError::WordStore(_) | SessionError::Base(_) => PyRuntimeError::new_err(message),
    }
}

/// The save gate's comparison as Python receives it: new problems, fixed ones, how many
/// the document opened with.
type Validation = (Vec<String>, Vec<String>, usize);

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
                ..CallContext::default()
            },
        })
    }
}

fn context_of(context: Option<&PyCallContext>) -> CallContext {
    context.map(|c| c.inner.clone()).unwrap_or_default()
}

/// A document at one version (`ooxml_core.Version`), held while Python holds it: a call
/// given it (`Session.call(..., at=[version])`) reads that version, whatever was
/// committed since.
#[pyclass(frozen, name = "Version", module = "ooxml_core")]
pub struct PyVersion {
    inner: Opened,
}

#[pymethods]
impl PyVersion {
    /// The document's id.
    #[getter]
    fn doc(&self) -> String {
        self.inner.id().to_string()
    }

    /// The version's number.
    #[getter]
    fn number(&self) -> u64 {
        self.inner.version()
    }

    fn __repr__(&self) -> String {
        format!("<ooxml_core.Version {} {}>", self.inner.id(), self.inner.version())
    }

    /// The parts whose content differs from `before` (a version of the same document):
    /// ``{part: bytes here, or None where this version lacks it}`` -- what a copy of the
    /// document kept beside the session takes to follow an edit made in it.
    fn changed_since<'py>(&self, py: Python<'py>, before: PyRef<'_, PyVersion>) -> PyResult<Bound<'py, PyDict>> {
        let (now, then) = (&self.inner, &before.inner);
        let changed = py.detach(|| now.changed_since(then)).map_err(|e| to_py(&e))?;
        let out = PyDict::new(py);
        for (name, bytes) in changed {
            match bytes {
                Some(bytes) => out.set_item(name, PyBytes::new(py, &bytes))?,
                None => out.set_item(name, py.None())?,
            }
        }
        Ok(out)
    }

    /// The names of the parts whose content differs from `before`: what
    /// [`changed_since`](Self::changed_since) gives, without their bytes.
    fn parts_changed_since(&self, py: Python<'_>, before: PyRef<'_, PyVersion>) -> PyResult<Vec<String>> {
        let (now, then) = (&self.inner, &before.inner);
        py.detach(|| now.parts_changed_since(then)).map_err(|e| to_py(&e))
    }

    /// The document's bytes at this version (untouched entries copied raw): what a reader of
    /// a version the host committed earlier is made from.
    fn save<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let version = &self.inner;
        let bytes = py
            .detach(|| version.package().save())
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(PyBytes::new(py, &bytes))
    }

    /// What changed since `before`, as docx-agent's ``Result.touched`` entries (JSON text).
    fn touched_since(&self, py: Python<'_>, before: PyRef<'_, PyVersion>) -> PyResult<String> {
        let (now, then) = (&self.inner, &before.inner);
        let found = py.detach(|| now.touched_since(then)).map_err(|e| to_py(&e))?;
        Ok(Value::Array(found).to_string())
    }

    /// The ids this version gave that `before` had not, and its renames that `before`
    /// lacks: ``(values, {old id: id now})`` -- what a view of the document kept beside the
    /// session takes to give no id twice and follow renamed ids, without reading a part.
    fn ids_since(&self, py: Python<'_>, before: PyRef<'_, PyVersion>) -> PyResult<(Vec<u32>, Vec<(String, String)>)> {
        let (now, then) = (&self.inner, &before.inner);
        let found = py.detach(|| now.ids_since(then)).map_err(|e| to_py(&e))?;
        Ok((found.given, found.renamed))
    }
}

/// What `Session.call` answers: the reply's JSON text, its images, and what only the
/// application reads (JSON text).
type Reply = (String, Vec<Py<PyBytes>>, String);

/// Python's placer of blocks (`placer(asked_json) -> answers_json | None`), called with the
/// interpreter lock taken back.
struct PyPlacer(Py<PyAny>);

impl Placer for PyPlacer {
    fn place(&self, asked: &[Vec<String>]) -> Option<Vec<Map<String, Value>>> {
        let asked = serde_json::to_string(asked).ok()?;
        let answer = Python::attach(|py| {
            let answer = self.0.call1(py, (asked,)).ok()?;
            answer.extract::<Option<String>>(py).ok().flatten()
        })?;
        let answers: Vec<Value> = serde_json::from_str(&answer).ok()?;
        answers
            .into_iter()
            .map(|a| match a {
                Value::Object(map) => Some(map),
                _ => None,
            })
            .collect()
    }
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
    /// it --, the images beside it (bytes), and what only the application reads, as JSON
    /// text: `unported`, what of the answer is not yet given as the Python tool gives it
    /// (a list of reasons). Never raises for the call's own failure:
    /// that is the reply's `error`, with its way out. `at`: the versions the call reads
    /// (`Version`s; a document not among them is read at its head). `placer`: where it may
    /// ask where blocks are laid out -- called with the blocks asked as JSON text (a list,
    /// each an id then the ids it joins), it answers each one's `placements` or `unknown`
    /// as JSON text (a list of objects), or `None` where it cannot.
    #[pyo3(signature = (name, arguments, *, context = None, limit = None, at = None, placer = None))]
    #[allow(clippy::too_many_arguments)]
    fn call(
        &self,
        py: Python<'_>,
        name: &str,
        arguments: String,
        context: Option<PyRef<'_, PyCallContext>>,
        limit: Option<usize>,
        at: Option<Vec<PyRef<'_, PyVersion>>>,
        placer: Option<Py<PyAny>>,
    ) -> Reply {
        let mut context = context_of(context.as_deref());
        context.at = at.iter().flatten().map(|v| v.inner.clone()).collect();
        context.placer = placer.map(|p| Arc::new(PyPlacer(p)) as Arc<dyn Placer>);
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
            let app = serde_json::json!({"unported": result.unported()});
            let text = match limit {
                Some(limit) => result.to_text(Some(limit)),
                None => compact(&Value::Object(result.to_json())),
            };
            (text, compact(&app))
        });
        (text.0, Vec::new(), text.1)
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
        let copied = copy_parts(parts)?;
        let session = &self.inner;
        let updated = py
            .detach(|| session.update(&id, copied, &context))
            .map_err(|e| to_py(&e))?;
        Ok((updated.version, updated.changed, updated.attempts))
    }

    /// Commit a version's touched parts as [`update`](Self::update) does: returns the
    /// version it leaves, held (`Version`; the version it ran on where nothing changed).
    #[pyo3(signature = (doc, parts, *, context = None))]
    fn commit(
        &self,
        py: Python<'_>,
        doc: &str,
        parts: &Bound<'_, PyDict>,
        context: Option<PyRef<'_, PyCallContext>>,
    ) -> PyResult<PyVersion> {
        let id = doc_id(doc, &self.inner)?;
        let context = context_of(context.as_deref());
        let copied = copy_parts(parts)?;
        let session = &self.inner;
        let (_, opened) = py
            .detach(|| session.commit(&id, copied, &context))
            .map_err(|e| to_py(&e))?;
        Ok(PyVersion { inner: opened })
    }

    /// The bridge: commit the parts the unported tool `tool` changed (``None`` removes one)
    /// and count the call in ``bridge_calls``; returns the version it leaves, held.
    #[pyo3(signature = (doc, tool, parts, *, context = None))]
    fn bridge(
        &self,
        py: Python<'_>,
        doc: &str,
        tool: &str,
        parts: &Bound<'_, PyDict>,
        context: Option<PyRef<'_, PyCallContext>>,
    ) -> PyResult<PyVersion> {
        let id = doc_id(doc, &self.inner)?;
        let context = context_of(context.as_deref());
        let copied = copy_parts(parts)?;
        let session = &self.inner;
        let (_, opened) = py
            .detach(|| session.bridge(&id, tool, copied, &context))
            .map_err(|e| to_py(&e))?;
        Ok(PyVersion { inner: opened })
    }

    /// ``bridge_calls{tool}``: the calls of each unported tool committed through the bridge.
    fn bridge_calls(&self, py: Python<'_>) -> Vec<(String, u64)> {
        let session = &self.inner;
        py.detach(|| session.bridge_calls().into_iter().collect())
    }

    /// The document's head version, held (`Version`). Never waits for a writer.
    fn head(&self, py: Python<'_>, doc: &str) -> PyResult<PyVersion> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        let opened = py.detach(|| session.snapshot(&id)).map_err(|e| to_py(&e))?;
        Ok(PyVersion { inner: opened })
    }

    /// The document's bytes at its head version. `checked`: through the save gate, refused
    /// (`ValueError`) when a Word document has validation problems it did not open with.
    #[pyo3(signature = (doc, *, checked = false))]
    fn save<'py>(&self, py: Python<'py>, doc: &str, checked: bool) -> PyResult<Bound<'py, PyBytes>> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        let bytes = py
            .detach(|| {
                if checked {
                    session.save_checked(&id)
                } else {
                    session.save(&id)
                }
            })
            .map_err(|e| to_py(&e))?;
        Ok(PyBytes::new(py, &bytes))
    }

    /// What the save gate compares for a Word document: `(new, fixed, baseline)`, the
    /// problems now that it did not open with, those it opened with that are gone, and how
    /// many it opened with; `None` for another kind.
    fn validation(&self, py: Python<'_>, doc: &str) -> PyResult<Option<Validation>> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        let delta = py.detach(|| session.validation(&id)).map_err(|e| to_py(&e))?;
        Ok(delta.map(|d| (d.new, d.fixed, d.baseline)))
    }

    /// Undo up to `steps` calls (with a caller in `context`, only its own, while its latest
    /// is the document's latest): `(count, version)`. A caller's latest change that others
    /// changed after raises `ValueError` when nothing was undone.
    #[pyo3(signature = (doc, steps = 1, *, context = None))]
    fn undo(
        &self,
        py: Python<'_>,
        doc: &str,
        steps: usize,
        context: Option<PyRef<'_, PyCallContext>>,
    ) -> PyResult<(usize, u64)> {
        let id = doc_id(doc, &self.inner)?;
        let context = context_of(context.as_deref());
        let session = &self.inner;
        let done = py
            .detach(|| session.undo(&id, steps, &context))
            .map_err(|e| to_py(&e))?;
        Ok((done.count, done.version))
    }

    /// Redo up to `steps` undone calls (with a caller, its own): `(count, version)`.
    #[pyo3(signature = (doc, steps = 1, *, context = None))]
    fn redo(
        &self,
        py: Python<'_>,
        doc: &str,
        steps: usize,
        context: Option<PyRef<'_, PyCallContext>>,
    ) -> PyResult<(usize, u64)> {
        let id = doc_id(doc, &self.inner)?;
        let context = context_of(context.as_deref());
        let session = &self.inner;
        let done = py
            .detach(|| session.redo(&id, steps, &context))
            .map_err(|e| to_py(&e))?;
        Ok((done.count, done.version))
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

/// Parts by name: bytes to put, or `None` to remove.
type Parts = Vec<(String, Option<Arc<[u8]>>)>;

/// `parts` (name -> bytes, or `None` to remove) copied in, the interpreter lock held.
fn copy_parts(parts: &Bound<'_, PyDict>) -> PyResult<Parts> {
    let mut copied = Vec::with_capacity(parts.len());
    for (name, data) in parts.iter() {
        let name: String = name.extract()?;
        let data: Option<Vec<u8>> = data.extract()?;
        copied.push((name, data.map(Arc::<[u8]>::from)));
    }
    Ok(copied)
}

/// The tools' definitions for `kind` as JSON text (a list of `ToolSpec` exports): what a
/// session made now would serve.
#[pyfunction]
pub fn tool_specs(py: Python<'_>, kind: &str) -> PyResult<String> {
    let kind = kind_of(kind)?;
    Ok(py.detach(|| compact(&Value::Array(Registry::standard().export(kind)))))
}
