//! Owns: `ooxml_core.Session` (the handles a call runs with are [`crate::context`]'s and
//! [`crate::options`]'s, a document's version [`crate::version`]'s): its Python signatures, the
//! copying of Python values into Rust ones while the interpreter
//! lock is held, every session call made with it released, and the mapping of errors.
//!
//! Must not: do a session's work itself, or touch a Python object without the lock.

use crate::adapters::{context, SessionStop};
use crate::context::{caller_context, PyCallContext};
use crate::locale::locale_of;
use crate::options::{context_of, PyCallLayout, PyCallOptions, PyHostIds};
use crate::version::PyVersion;
use ooxml_base::Locale;
use ooxml_session::{DocId, DocKind, HeldId, RetiredBy, Session, SessionConfig, SessionError};
use ooxml_store::StoreConfig;
use ooxml_tools::compact;
use ooxml_word_edit::EarlierIds;
use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyTimeoutError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use serde_json::Value;
use std::sync::Arc;

/// Documents by id, their versions, and the tools that read them. Every method releases
/// the interpreter lock for its Rust work; any number of threads may use one session.
#[pyclass(frozen, name = "Session", module = "ooxml_core")]
pub struct PySession {
    inner: Arc<Session>,
    stop: Arc<SessionStop>,
}

pub(crate) fn kind_of(kind: &str) -> PyResult<DocKind> {
    DocKind::parse(kind).ok_or_else(|| to_py(&SessionError::Kind(kind.to_owned())))
}

pub(crate) fn doc_id(doc: &str, session: &Session) -> PyResult<DocId> {
    DocId::parse(doc).ok_or_else(|| {
        to_py(&SessionError::NotOpen {
            id: doc.to_owned(),
            open: session.documents().iter().map(ToString::to_string).collect(),
        })
    })
}

pub(crate) fn to_py(error: &SessionError) -> PyErr {
    let message = error.to_string();
    match error {
        SessionError::NotOpen { .. } => PyKeyError::new_err(message),
        SessionError::Kind(_) | SessionError::PartName(_) | SessionError::Package(_) => PyValueError::new_err(message),
        SessionError::Base(ooxml_base::BaseError::Cancelled) => PyTimeoutError::new_err(message),
        SessionError::Word(_) => PyValueError::new_err(message),
        SessionError::Entangled { .. } | SessionError::RedoNotNext { .. } | SessionError::NewProblems { .. } => {
            PyValueError::new_err(message)
        }
        SessionError::Store(_) | SessionError::WordStore(_) | SessionError::Base(_) => PyRuntimeError::new_err(message),
    }
}

/// The save gate's comparison as Python receives it: new problems, fixed ones, how many
/// the document opened with.
type Validation = (Vec<String>, Vec<String>, usize);

/// A step as `Session.numbers` lists it: `(before, after, seq, caller)`.
type NumberedRow = (u64, u64, u64, Option<String>);
/// `Session.numbers`: `(highest, steps, redo)`.
type HistoryNumbers = (u64, Vec<NumberedRow>, Vec<NumberedRow>);

/// What `Session.edit` says a call made: its first version's number, its last version,
/// what it touched (JSON text; `None` where it made more than one version), the last
/// version's validation, the state number of the version its first edit was applied to, and
/// the reflow its checks report (JSON text; `None` where none was measured).
type EditMade = (u64, PyVersion, Option<String>, Validation, u64, Option<String>);

/// A Word document's ids before it was saved, as a host's state keeps them:
/// `(values, cursors, highest_drawing, drawings)` ([`EarlierIds`]).
type StateIds = (Vec<u32>, Vec<(String, usize)>, u64, Vec<u64>);

/// One retired id as `Session.retired` gives it: `(kind, value, seq, caller, tool, version,
/// said)` -- `"block"` (a paraId's value) or `"drawing"`; a version this session made
/// (`seq`, its caller's key and its tool) or, from a state the document was opened with, the
/// host's `version` and words.
type RetiredRow = (
    &'static str,
    u64,
    Option<u64>,
    Option<String>,
    Option<String>,
    Option<u64>,
    Option<String>,
);

fn retired_row((id, by): (HeldId, RetiredBy)) -> RetiredRow {
    let (kind, value) = match id {
        HeldId::Block(v) => ("block", u64::from(v)),
        HeldId::Drawing(n) => ("drawing", n),
    };
    match by {
        RetiredBy::Step { seq, caller, tool } => (
            kind,
            value,
            Some(seq),
            caller.map(|c| c.to_string()),
            tool.map(|t| t.to_string()),
            None,
            None,
        ),
        RetiredBy::Earlier { version, said } => (kind, value, None, None, None, Some(version), said),
    }
}

/// An id `Session.seed_retired` is given: `"block"` or `"drawing"` and its value.
fn held_id(kind: &str, value: u64) -> PyResult<HeldId> {
    match kind {
        "block" => u32::try_from(value)
            .map(HeldId::Block)
            .map_err(|_| PyValueError::new_err(format!("a block id's value is at most 0xFFFFFFFF, not {value}"))),
        "drawing" => Ok(HeldId::Drawing(value)),
        _ => Err(PyValueError::new_err(format!(
            "an id's kind is 'block' or 'drawing', not {kind:?}"
        ))),
    }
}

/// What `Session.call` answers: the reply's JSON text, its images, and what only the
/// application reads (JSON text).
type Reply = (String, Vec<Py<PyBytes>>, String);

#[pymethods]
impl PySession {
    /// A session: each document keeps `history_depth` undo steps; `locale` (ooxml-common's
    /// `Locale` or a BCP 47 tag) is the one the session's documents are read under, unless
    /// a call names its own; `None`: en-US, and each document's own language where a call
    /// reads one (a table of contents' style names).
    #[new]
    #[pyo3(signature = (*, history_depth = 50, locale = None))]
    fn new(history_depth: usize, locale: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let locale = locale.map(locale_of).transpose()?;
        let tag = locale.as_ref().map(Locale::from).unwrap_or_default();
        let stop = Arc::new(SessionStop::default());
        let config = SessionConfig {
            store: StoreConfig {
                history_depth,
                ..StoreConfig::default()
            },
            locale,
        };
        let inner = Session::new(config, context(Arc::clone(&stop), tag));
        Ok(Self {
            inner: Arc::new(inner),
            stop,
        })
    }

    /// Open a package's bytes as a document of `kind`, named `name`; returns its id (`d1`).
    /// `ids`: a Word document's ids before it was saved, as the host's state of it keeps them
    /// -- `(values, cursors, highest_drawing, drawings)`: every value its pool held, each
    /// seed's cursor, the highest drawing number given, the drawing numbers retired -- so it
    /// never gives a retired id or drawing number again.
    #[pyo3(signature = (data, *, kind = "docx", name = "document.docx", ids = None))]
    fn open(&self, py: Python<'_>, data: &[u8], kind: &str, name: &str, ids: Option<StateIds>) -> PyResult<String> {
        let kind = kind_of(kind)?;
        let bytes: Arc<[u8]> = Arc::from(data);
        let earlier = ids.map_or_else(EarlierIds::default, |(values, cursors, highest_drawing, drawings)| {
            EarlierIds {
                values,
                cursors,
                highest_drawing,
                drawings,
            }
        });
        let session = &self.inner;
        let id = py
            .detach(|| session.open_with(bytes, kind, name, &earlier))
            .map_err(|e| to_py(&e))?;
        Ok(id.to_string())
    }

    /// Run the tool `name` with `arguments` (JSON text) on a snapshot, with `options` (a
    /// `CallOptions`: who calls, the versions read, the placer, the page engine, the reply's
    /// `limit`, ...): returns the reply as JSON text -- the whole envelope, or within the
    /// limit as the model is sent it --, the files the call gives the application (bytes:
    /// `save_document`'s), and what only the application reads, as JSON text: `unported`,
    /// what of the answer is not yet given as the Python tool gives it (a list of reasons),
    /// `files`, each file's `name` and `format` in the order of the bytes, and `panic`, what a
    /// call that panicked said (else `null`; its reply is an `internal` failure). Never raises
    /// for the call's own failure: that is the reply's `error`, with its way out.
    #[pyo3(signature = (name, arguments, options = None))]
    fn call(&self, py: Python<'_>, name: &str, arguments: String, options: Option<PyRef<'_, PyCallOptions>>) -> Reply {
        let context = context_of(options.as_deref());
        let limit = options.as_ref().and_then(|o| o.limit);
        let session = &self.inner;
        let (text, files, app) = py.detach(|| {
            let mut result = session.call(name, Value::String(arguments), &context);
            let files = std::mem::take(&mut result.app.files);
            let named: Vec<Value> = files
                .iter()
                .map(|f| serde_json::json!({"name": f.name, "format": f.format}))
                .collect();
            let app = serde_json::json!({"unported": result.unported(), "files": named, "panic": result.app.panic});
            let text = match limit {
                Some(limit) => result.to_text(Some(limit)),
                None => compact(&Value::Object(result.to_json())),
            };
            (text, files, compact(&app))
        });
        let files = files.iter().map(|f| PyBytes::new(py, &f.bytes).unbind()).collect();
        (text, files, app)
    }

    /// Run the editing tool `name` as [`call`](Self::call) does, with `options` (a
    /// `CallOptions`: who calls, the page engine, the blobs, the values written by reference;
    /// no limit, no images): returns the envelope's JSON text, the application's JSON text
    /// (`unported`, `panic`), and what the call's edits made -- `(first, last, touched, validation,
    /// before, reflow)`: the number of the version its first edit made, the version its last
    /// made (held), what it touched as JSON text (`None` where it made more than one version),
    /// the last version's validation as [`validation`](Self::validation) gives it, the state
    /// number of the version its first edit was applied to, and the reflow its checks report as
    /// JSON text (where the options' page engine laid out the version its first edit was
    /// applied to; else `None`) -- or `None` where it changed nothing. A host that numbers the
    /// document's steps takes the call's step from these, whatever other calls commit to the
    /// document meanwhile.
    ///
    /// # Errors
    /// `ValueError` for options with a `limit` (an edit's reply is whole).
    #[pyo3(signature = (name, arguments, options = None))]
    fn edit(
        &self,
        py: Python<'_>,
        name: &str,
        arguments: String,
        options: Option<PyRef<'_, PyCallOptions>>,
    ) -> PyResult<(String, String, Option<EditMade>)> {
        if options.as_ref().is_some_and(|o| o.limit.is_some()) {
            return Err(PyValueError::new_err(
                "an edit's reply is whole: give its options no limit",
            ));
        }
        let context = context_of(options.as_deref());
        let session = &self.inner;
        let (text, app, made) = py.detach(|| {
            let (result, made) = session.edit(name, Value::String(arguments), &context);
            let app = serde_json::json!({"unported": result.unported(), "panic": result.app.panic});
            (compact(&Value::Object(result.to_json())), compact(&app), made)
        });
        let made = made.map(|m| {
            let touched = m.touched.map(|t| Value::Array(t).to_string());
            let validation = (m.validation.new, m.validation.fixed, m.validation.baseline);
            let reflow = m.reflow.map(|r| r.to_string());
            (
                m.first,
                PyVersion { inner: m.last },
                touched,
                validation,
                m.before,
                reflow,
            )
        });
        Ok((text, app, made))
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
        let context = caller_context(context.as_deref());
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
        let context = caller_context(context.as_deref());
        let copied = copy_parts(parts)?;
        let session = &self.inner;
        let (_, opened) = py
            .detach(|| session.commit(&id, copied, &context))
            .map_err(|e| to_py(&e))?;
        Ok(PyVersion { inner: opened })
    }

    /// The bridge: commit the parts the unported tool `tool` changed (``None`` removes one)
    /// and count the call in ``bridge_calls``; ``ids``: what the host's id ledger holds
    /// beside the parts (a ``HostIds``: its tool's draws and renames). Returns the version
    /// it leaves, held.
    #[pyo3(signature = (doc, tool, parts, *, context = None, ids = None))]
    fn bridge(
        &self,
        py: Python<'_>,
        doc: &str,
        tool: &str,
        parts: &Bound<'_, PyDict>,
        context: Option<PyRef<'_, PyCallContext>>,
        ids: Option<PyRef<'_, PyHostIds>>,
    ) -> PyResult<PyVersion> {
        let id = doc_id(doc, &self.inner)?;
        let context = caller_context(context.as_deref());
        let copied = copy_parts(parts)?;
        let host = ids.map(|i| i.inner.clone()).unwrap_or_default();
        let session = &self.inner;
        let (_, opened) = py
            .detach(|| session.bridge(&id, tool, copied, host, &context))
            .map_err(|e| to_py(&e))?;
        Ok(PyVersion { inner: opened })
    }

    /// ``bridge_calls{tool}``: the calls of each unported tool committed through the bridge.
    fn bridge_calls(&self, py: Python<'_>) -> Vec<(String, u64)> {
        let session = &self.inner;
        py.detach(|| session.bridge_calls().into_iter().collect())
    }

    /// Open a step group on the document (`StepGroup`): what is committed until it closes is
    /// one undo step; the host closes or reverts it on every path.
    fn open_group(&self, py: Python<'_>, doc: &str) -> PyResult<crate::group::PyStepGroup> {
        let id = doc_id(doc, &self.inner)?;
        crate::group::PyStepGroup::open(py, Arc::clone(&self.inner), id)
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
    /// `at`: of that version (a `Version` of the document) instead of the head.
    #[pyo3(signature = (doc, *, at = None))]
    fn validation(&self, py: Python<'_>, doc: &str, at: Option<PyRef<'_, PyVersion>>) -> PyResult<Option<Validation>> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        let at = at.map(|v| v.inner.clone());
        let delta = py
            .detach(|| match &at {
                Some(at) => session.validation_at(at),
                None => session.validation(&id),
            })
            .map_err(|e| to_py(&e))?;
        Ok(delta.map(|d| (d.new, d.fixed, d.baseline)))
    }

    /// What the document's last check with `reflow` saw, which the session's next check
    /// measures `since_last_check` from: `(version, summary)` -- the number of the version
    /// it saw (`None`: the host's check kept it without one) and its `LayoutSummary` (of a
    /// check the session served, the version laid out: summarised when read or compared) --
    /// or `None`.
    fn last_check(&self, doc: &str) -> PyResult<Option<(Option<u64>, crate::summary::PyLayoutSummary)>> {
        let id = doc_id(doc, &self.inner)?;
        let kept = self.inner.last_check(&id).map_err(|e| to_py(&e))?;
        Ok(kept
            .and_then(|kept| kept.get())
            .map(|seen| (seen.version, crate::summary::PyLayoutSummary { inner: seen.pages })))
    }

    /// What the document's last check that laid it out saw, with `reflow` or not (a check
    /// of the layout alone, of ranges): `(version, summary)` as [`Self::last_check`] gives
    /// it, or `None` -- what a changing call is measured against before its change.
    fn last_laid(&self, doc: &str) -> PyResult<Option<(Option<u64>, crate::summary::PyLayoutSummary)>> {
        let id = doc_id(doc, &self.inner)?;
        let kept = self.inner.last_check(&id).map_err(|e| to_py(&e))?;
        Ok(kept
            .and_then(|kept| kept.laid())
            .map(|seen| (seen.version, crate::summary::PyLayoutSummary { inner: seen.pages })))
    }

    /// Keep `summary` (a `LayoutSummary` of the document's engine) as what the document's
    /// last check saw -- a check the host made itself -- of the version `at` where given;
    /// `None`: forget what the last check saw.
    #[pyo3(signature = (doc, summary = None, *, at = None))]
    fn set_last_check(
        &self,
        doc: &str,
        summary: Option<PyRef<'_, crate::summary::PyLayoutSummary>>,
        at: Option<PyRef<'_, PyVersion>>,
    ) -> PyResult<()> {
        let id = doc_id(doc, &self.inner)?;
        let seen = summary.map(|summary| ooxml_tools_word::Seen {
            version: at.map(|v| v.inner.version()),
            pages: summary.inner.clone(),
        });
        if let Some(kept) = self.inner.last_check(&id).map_err(|e| to_py(&e))? {
            kept.set(seen);
        }
        Ok(())
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
        let context = caller_context(context.as_deref());
        let session = &self.inner;
        let done = py
            .detach(|| session.undo(&id, steps, &context))
            .map_err(|e| to_py(&e))?;
        Ok((done.count, done.version))
    }

    /// The document's state numbered `number` (`Version.state`) its head reaches, held: the
    /// head, else the newest such state undo or redo can bring back; `None` when none is kept.
    fn numbered(&self, py: Python<'_>, doc: &str, number: u64) -> PyResult<Option<PyVersion>> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        let found = py.detach(|| session.numbered(&id, number)).map_err(|e| to_py(&e))?;
        Ok(found.map(|inner| PyVersion { inner }))
    }

    /// Go on with the state numbers the host gave the document before it was opened again:
    /// its head is the state `state`, the next step's number one more than `highest`. Only
    /// before its first step (`ValueError` otherwise, or for a state above the highest).
    fn continue_numbers(&self, py: Python<'_>, doc: &str, state: u64, highest: u64) -> PyResult<()> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        py.detach(|| session.continue_numbers(&id, state, highest))
            .map_err(|e| to_py(&e))
    }

    /// The highest state number given on the document (`Version.state`). Never waits.
    fn highest(&self, py: Python<'_>, doc: &str) -> PyResult<u64> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        py.detach(|| session.highest(&id)).map_err(|e| to_py(&e))
    }

    /// The document's history in state numbers, read at once: `(highest, steps, redo)` --
    /// the highest number given, the steps undo can reach (oldest first) and those redo can
    /// bring back (the next one last), each `(before, after, seq, caller)`.
    fn numbers(&self, py: Python<'_>, doc: &str) -> PyResult<HistoryNumbers> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        let numbers = py.detach(|| session.numbers(&id)).map_err(|e| to_py(&e))?;
        let rows = |steps: Vec<ooxml_session::NumberedStep>| {
            steps
                .into_iter()
                .map(|s| (s.before, s.after, s.seq, s.caller))
                .collect()
        };
        Ok((numbers.highest, rows(numbers.steps), rows(numbers.redo)))
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
        let context = caller_context(context.as_deref());
        let session = &self.inner;
        let done = py
            .detach(|| session.redo(&id, steps, &context))
            .map_err(|e| to_py(&e))?;
        Ok((done.count, done.version))
    }

    /// The Word document's retired ids: each block id (a paraId's value) and drawing number a
    /// version removed and none brought back since, with who removed it, as rows
    /// `(kind, value, seq, caller, tool, version, said)` (see `seed_retired` for the last two).
    fn retired(&self, py: Python<'_>, doc: &str) -> PyResult<Vec<RetiredRow>> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        let rows = py.detach(|| session.retired(&id)).map_err(|e| to_py(&e))?;
        Ok(rows.into_iter().map(retired_row).collect())
    }

    /// Give the Word document the ids its saved state retired before it was opened again:
    /// rows `(kind, value, version, said)`, the version that removed each as the host numbered
    /// it and who made it in the host's words (`None`: not known).
    fn seed_retired(&self, py: Python<'_>, doc: &str, rows: Vec<(String, u64, u64, Option<String>)>) -> PyResult<()> {
        let id = doc_id(doc, &self.inner)?;
        let given = rows
            .into_iter()
            .map(|(kind, value, version, said)| Ok((held_id(&kind, value)?, version, said)))
            .collect::<PyResult<Vec<_>>>()?;
        let session = &self.inner;
        py.detach(|| session.seed_retired(&id, given)).map_err(|e| to_py(&e))
    }

    /// The host numbered the Word document's `version` (a `Version` of it) as its version
    /// `number`: a refusal of an id a version up to it removed names the change by it.
    fn number_version(&self, py: Python<'_>, doc: &str, version: PyRef<'_, PyVersion>, number: u64) -> PyResult<()> {
        let id = doc_id(doc, &self.inner)?;
        let seq = version.inner.version();
        let session = &self.inner;
        py.detach(|| session.number_version(&id, seq, number))
            .map_err(|e| to_py(&e))
    }

    /// The rows of the Word document this session's `word_edit_table` calls added or wrote
    /// into, by `w14:paraId`, sorted (round 58, R126: check's `row_text_format` reads them).
    fn rows_written(&self, py: Python<'_>, doc: &str) -> PyResult<Vec<String>> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        let rows = py.detach(|| session.rows_written(&id)).map_err(|e| to_py(&e))?;
        Ok(rows.into_iter().collect())
    }

    /// Keep `rows` (`w14:paraId`s) as written by a table call the host answered itself, in
    /// the record the session's own calls keep theirs in.
    fn note_rows_written(&self, py: Python<'_>, doc: &str, rows: Vec<String>) -> PyResult<()> {
        let id = doc_id(doc, &self.inner)?;
        let session = &self.inner;
        py.detach(|| session.note_rows_written(&id, rows))
            .map_err(|e| to_py(&e))
    }

    /// The live view of the Word document `doc` (an application's API, not a tool): its
    /// pages at its head version, laid out by `layout`'s page engine (a `CallLayout(pages=)`)
    /// within `context`'s limit on laying out, and those changed since the view whose token is
    /// `since` (every page without one). Returns a dict: `token` (this version's: `since` for
    /// the next view, `page_svg`'s token), `pages` (each `(number, blocks, width, height)`:
    /// the ids of the body's top-level blocks it draws, in document order, its size in
    /// points), `changed` (1-based), `since` (as given), `stale` (`since` names no version
    /// kept: every page is changed), `note`, `version` (the state number laid out).
    ///
    /// # Errors
    /// `KeyError` for a document not open; `ValueError` for one that is not Word's or a
    /// layout without pages; `ooxml_core.ViewRefused` for a version the engine does not lay
    /// out yet; `TimeoutError` past the limit.
    #[pyo3(signature = (doc, since = None, *, layout, context = None))]
    fn view<'py>(
        &self,
        py: Python<'py>,
        doc: &str,
        since: Option<&str>,
        layout: PyRef<'_, PyCallLayout>,
        context: Option<PyRef<'_, PyCallContext>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        crate::view::view(py, &self.inner, (doc, since), &layout, context.as_deref())
    }

    /// Page `page` (1-based) of the version `token` names (a view's) as SVG, its paragraph
    /// groups carrying `data-docx-agent-id` (that version's ids), its pictures embedded:
    /// `(svg, warnings)`. `text`: `"text"` (`"outlines"` is not drawn yet).
    ///
    /// # Errors
    /// `ooxml_core.UnknownToken` for a token no view of the document gave or kept;
    /// `IndexError` for a page the version does not have; `ValueError` for a `text` there is
    /// none of; `ooxml_core.ViewRefused` for `"outlines"`; as [`view`](Self::view).
    #[pyo3(signature = (doc, page, token, *, layout, text = "text", context = None))]
    fn page_svg(
        &self,
        doc: &str,
        page: usize,
        token: &str,
        layout: PyRef<'_, PyCallLayout>,
        text: &str,
        context: Option<PyRef<'_, PyCallContext>>,
    ) -> PyResult<(String, Vec<String>)> {
        let py = layout.py();
        crate::view::page_svg(py, &self.inner, (doc, page, token), (&layout, text), context.as_deref())
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

    /// The session's counters: calls, failed, cancelled, updates, open, panics (the calls
    /// that panicked, of every tool: [`panics`](Self::panics)).
    fn stats(&self, py: Python<'_>) -> Vec<(&'static str, u64)> {
        let s = py.detach(|| self.inner.stats());
        vec![
            ("calls", s.calls),
            ("failed", s.failed),
            ("cancelled", s.cancelled),
            ("updates", s.updates),
            ("open", s.open),
            ("panics", s.panics.values().sum()),
        ]
    }

    /// ``panics{tool}``: the calls of each tool that panicked, each answered as an
    /// ``internal`` failure (its application part's ``panic`` says what it said).
    fn panics(&self, py: Python<'_>) -> Vec<(String, u64)> {
        py.detach(|| self.inner.stats().panics.into_iter().collect())
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
