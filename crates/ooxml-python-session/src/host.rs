//! Owns: the Rust host for Python (`ooxml_core.Host`, one per toolbox, and
//! `ooxml_core.HostSession`, one per application session: design phase 3, decision D7) --
//! its Python signatures; the application's settings read as values (ooxml-edit's `Limits`
//! by field name, the roles, the rules of `check`'s numbers, docx-agent's format settings);
//! every call made with the interpreter lock let go; and its answers handed back as JSON
//! text and bytes.
//!
//! Must not: call back into Python (the clock's date of a call, the blobs, the sources'
//! values all come in as values), hold a Python object, or do a host's work itself.

use crate::adapters::{context, SessionStop};
use crate::context::PyCallContext;
use crate::layout::PyWordPages;
use crate::locale::locale_of;
use crate::options::PyCallLayout;
use crate::pages::PyPageSetup;
use crate::session::to_py;
use crate::version::PyVersion;
use ooxml_host::{CallAs, Host, HostSession, HostSettings, HostTools, Opening};
use ooxml_session::numbers::NumberRules;
use ooxml_session::{DocId, DocKind, Format, PackageFormat, SessionConfig};
use ooxml_session_word::prelude::*;
use ooxml_session_word::WordFormat;
use ooxml_store::StoreConfig;
use ooxml_tools::compact;
use ooxml_tools::roles::RoleSet;
use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use serde_json::{Map, Value};
use std::sync::Arc;

pyo3::create_exception!(
    ooxml_core,
    OpenRefused,
    PyValueError,
    "A document the host did not open: `args` is `(message, why)`, `why` one of `argument` \
(an id that is no id or is in use, a kind no format opens), `limit` (a package's or the \
session's room broken), `unreadable` (bytes no format reads) and `state` (the state given \
beside the bytes refused)."
);

fn refused(error: &ooxml_host::OpenRefused) -> PyErr {
    let why = match error {
        ooxml_host::OpenRefused::Argument(_) => "argument",
        ooxml_host::OpenRefused::Limit(_) => "limit",
        ooxml_host::OpenRefused::Unreadable(_) => "unreadable",
        ooxml_host::OpenRefused::State(_) => "state",
    };
    OpenRefused::new_err((error.to_string(), why))
}

fn json_of(text: &str, what: &str) -> PyResult<Value> {
    serde_json::from_str(text).map_err(|e| PyValueError::new_err(format!("{what}: {e}")))
}

fn object_of(text: Option<&str>, what: &str) -> PyResult<Map<String, Value>> {
    match text.map(|t| json_of(t, what)).transpose()? {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map),
        Some(_) => Err(PyValueError::new_err(format!("{what}: a JSON object"))),
    }
}

fn roles_of(names: Vec<String>) -> PyResult<RoleSet> {
    RoleSet::parse(names.iter().map(String::as_str)).map_err(PyValueError::new_err)
}

/// One host: Word's format (with the application's settings of it) and the kinds Rust holds
/// only as packages, the host's tools, and the application's settings, which every session
/// it makes runs with.
#[pyclass(frozen, name = "Host", module = "ooxml_core")]
pub struct PyHost {
    inner: Arc<Host>,
}

#[pymethods]
impl PyHost {
    /// A host with `settings` -- JSON text: `limits` (ooxml-edit's `Limits`, by field name;
    /// the fields only the Python host reads are left to it), `roles` (a list of role
    /// names), `log_arguments`, `allow_new_problems`, `numbers` (the rules of `check`'s
    /// numbers), `own_tools` (the application's own tools, which its facade runs: refused
    /// inside a batch) -- laying its Word documents out with `pages` (a `PageSetup`), docx-agent's
    /// `min_text_size` (JSON text: points, or points by kind) and `typography`.
    ///
    /// # Errors
    /// `ValueError` for a setting the host cannot take.
    #[new]
    #[pyo3(signature = (settings = None, *, pages = None, min_text_size = None, typography = true))]
    fn new(
        settings: Option<&str>,
        pages: Option<PyRef<'_, PyPageSetup>>,
        min_text_size: Option<&str>,
        typography: bool,
    ) -> PyResult<Self> {
        let given = object_of(settings, "settings")?;
        let mut host = HostSettings::default();
        for (key, value) in &given {
            match key.as_str() {
                "limits" => {
                    let limits = value
                        .as_object()
                        .ok_or_else(|| PyValueError::new_err("settings.limits: a JSON object"))?;
                    host.take_limits(limits).map_err(PyValueError::new_err)?;
                }
                "roles" => host.take_roles(value).map_err(PyValueError::new_err)?,
                "log_arguments" => host.log_arguments = value.as_bool().unwrap_or(false),
                "allow_new_problems" => host.allow_new_problems = value.as_bool().unwrap_or(false),
                "numbers" => host.numbers = NumberRules::from_json(value).map_err(PyValueError::new_err)?,
                "own_tools" => {
                    host.own_tools = value
                        .as_array()
                        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                        .ok_or_else(|| PyValueError::new_err("settings.own_tools: a list of tool names"))?;
                }
                other => return Err(PyValueError::new_err(format!("settings has no {other:?}"))),
            }
        }
        let minimum = match min_text_size {
            Some(text) => {
                ooxml_tools_word::minimum_of(&json_of(text, "min_text_size")?).map_err(PyValueError::new_err)?
            }
            None => None,
        };
        let word = WordFormat {
            pages: pages.map(|p| p.inner.clone()),
            min_text_size: minimum,
            as_given: !typography,
        };
        let formats: Vec<Arc<dyn Format>> = vec![
            Arc::new(word),
            Arc::new(PackageFormat(DocKind::Pptx)),
            Arc::new(PackageFormat(DocKind::Xlsx)),
        ];
        Ok(Self {
            inner: Arc::new(Host::new(formats, host, Arc::new(HostTools))),
        })
    }

    /// A new session: each document keeps `history_depth` undo steps; `locale` (ooxml-common's
    /// `Locale` or a BCP 47 tag) is the one its documents are read under unless a call names
    /// its own; `None`: each document's own language.
    #[pyo3(signature = (*, locale = None, history_depth = 50))]
    fn session(&self, locale: Option<&Bound<'_, PyAny>>, history_depth: usize) -> PyResult<PyHostSession> {
        let locale = locale.map(locale_of).transpose()?;
        let tag = locale.as_ref().map(ooxml_base::Locale::from).unwrap_or_default();
        let stop = Arc::new(SessionStop::default());
        let config = SessionConfig {
            store: StoreConfig {
                history_depth,
                ..StoreConfig::default()
            },
            locale,
        };
        let inner = self.inner.session(config, context(Arc::clone(&stop), tag));
        Ok(PyHostSession {
            inner: Arc::new(inner),
            stop,
        })
    }

    /// The tools' names, in the host's catalogue order.
    fn tools(&self) -> Vec<String> {
        self.inner
            .catalogue()
            .entries()
            .iter()
            .map(|e| e.name().to_owned())
            .collect()
    }
}

/// What `HostSession.call` answers: the reply's JSON text, its files `(name, format, bytes)`
/// (a `render`'s PNG pages among them), and what only the application reads (JSON text:
/// `touched`, `touched_by_op`, `changed_parts`, `timing`, `memory`, `budget`, `unchanged`,
/// `panic`, and the call's `record`).
type Reply = (String, Vec<(String, String, Py<PyBytes>)>, String);

/// One saved document: `(doc, name, kind, version, bytes, state)`.
type SavedRow = (String, String, &'static str, u64, Py<PyBytes>, Option<String>);

/// One file a tool saved for the application: `(name, format, doc, bytes, validate, state)`,
/// `validate` as JSON text.
type OutputRow = (
    String,
    String,
    Option<String>,
    Py<PyBytes>,
    Option<String>,
    Option<String>,
);

/// One application session of a host: its documents (`d1`...), blobs, outputs, image
/// budgets, datasets, refs, retry keys and log. Every method lets go of the interpreter lock
/// for its Rust work; any number of threads may use one session.
#[pyclass(frozen, name = "HostSession", module = "ooxml_core")]
pub struct PyHostSession {
    inner: Arc<HostSession>,
    stop: Arc<SessionStop>,
}

impl PyHostSession {
    fn doc(&self, doc: &str) -> PyResult<DocId> {
        DocId::parse(doc)
            .filter(|id| self.inner.session().kind_of(id).is_ok())
            .ok_or_else(|| {
                let open: Vec<String> = self
                    .inner
                    .session()
                    .documents()
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                PyKeyError::new_err(format!("no open document {doc:?}; open: {}", open.join(", ")))
            })
    }
}

#[pymethods]
impl PyHostSession {
    /// Open a package's bytes as `name`: its id (`d1`). `kind`: its format (`None`: found
    /// from the bytes and name); `source`: where it came from (`app`); `doc_id`: the id to
    /// open it under (a resumed run's); `state`: what was saved beside the bytes (text, or
    /// ASCII bytes).
    ///
    /// # Errors
    /// `OpenRefused`, in today's words; `StateRefused` for a state that is no text.
    #[pyo3(signature = (data, *, name, kind = None, source = None, doc_id = None, state = None))]
    #[allow(clippy::too_many_arguments)] // (Session.open's keywords, as Python passes them)
    fn open(
        &self,
        py: Python<'_>,
        data: &[u8],
        name: &str,
        kind: Option<&str>,
        source: Option<&str>,
        doc_id: Option<&str>,
        state: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<String> {
        let state = state.map(crate::session::state_text).transpose()?;
        let kind = match kind {
            Some(k) => Some(
                DocKind::parse(k)
                    .ok_or_else(|| refused(&ooxml_host::OpenRefused::Argument(format!("no format {k:?}"))))?,
            ),
            None => None,
        };
        let bytes: Arc<[u8]> = Arc::from(data);
        let how = Opening {
            kind,
            source,
            doc_id,
            state: state.as_deref(),
        };
        let session = &self.inner;
        py.detach(|| session.open_with(bytes, name, &how))
            .map(|id| id.to_string())
            .map_err(|e| refused(&e))
    }

    /// Register an input: its handle (`b1`).
    ///
    /// # Errors
    /// `ValueError`: a limit broken, or bytes that are not what they claim.
    #[pyo3(signature = (data, name, mime = None))]
    fn add_blob(&self, py: Python<'_>, data: &[u8], name: &str, mime: Option<&str>) -> PyResult<String> {
        let bytes: Arc<[u8]> = Arc::from(data);
        let session = &self.inner;
        py.detach(|| session.add_blob(bytes, name, mime))
            .map_err(|e| PyValueError::new_err(e.0))
    }

    /// A blob by its handle: `(name, mime, bytes)`, `None` where there is none.
    fn blob(&self, py: Python<'_>, handle: &str) -> Option<(String, String, Py<PyBytes>)> {
        let stores = self.inner.stores();
        let found = stores.blobs.blobs().iter().find(|b| b.handle == handle)?;
        Some((
            found.name.clone(),
            found.mime.clone(),
            PyBytes::new(py, &found.data).unbind(),
        ))
    }

    /// Every blob as `(handle, name, mime, size)`, in the order registered.
    fn blobs(&self) -> Vec<(String, String, String, usize)> {
        let stores = self.inner.stores();
        stores
            .blobs
            .blobs()
            .iter()
            .map(|b| (b.handle.clone(), b.name.clone(), b.mime.clone(), b.data.len()))
            .collect()
    }

    /// The open documents' ids, in id order.
    fn documents(&self) -> Vec<String> {
        self.inner
            .session()
            .documents()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// An open document's `(kind, name, version, file kind)`: the file kind a Word
    /// document's main part says (`docx`, `docm`, `dotx`, `dotm`), `None` for another.
    ///
    /// # Errors
    /// `KeyError` for a document not open.
    fn document(&self, py: Python<'_>, doc: &str) -> PyResult<(&'static str, String, u64, Option<&'static str>)> {
        let id = self.doc(doc)?;
        let head = self.inner.session().snapshot(&id).map_err(|e| to_py(&e))?;
        let file_kind = py.detach(|| head.file_kind()).ok().flatten();
        Ok((head.kind().as_str(), head.name().to_owned(), head.number(), file_kind))
    }

    /// The refs of the open document `doc` as they are now: `{name: address}` (JSON text).
    ///
    /// # Errors
    /// `KeyError` for a document not open.
    fn refs(&self, doc: &str) -> PyResult<String> {
        let id = self.doc(doc)?;
        let stores = self.inner.stores();
        let refs = stores
            .refs
            .get(&id.to_string())
            .map(|table| table.refs().to_json())
            .unwrap_or_default();
        Ok(compact(&Value::Object(refs)))
    }

    /// `doc` at its head version (an `ooxml_core.Version`, held while Python holds it).
    ///
    /// # Errors
    /// `KeyError` for a document not open.
    fn version(&self, doc: &str) -> PyResult<PyVersion> {
        let id = self.doc(doc)?;
        let inner = self.inner.session().snapshot(&id).map_err(|e| to_py(&e))?;
        Ok(PyVersion { inner })
    }

    /// The page engine `doc` is laid out with (a `WordPages`); `None` for a document that is
    /// not Word's or one the host lays out with nothing.
    ///
    /// # Errors
    /// `KeyError` for a document not open.
    fn pages(&self, doc: &str) -> PyResult<Option<PyWordPages>> {
        let id = self.doc(doc)?;
        let found = self.inner.session().pages(&id).map_err(|e| to_py(&e))?;
        Ok(found.as_deref().map(PyWordPages::of))
    }

    /// What the session keeps of `doc` in its memory budget now, bytes: its page engine's
    /// accounts (0 where it has none).
    ///
    /// # Errors
    /// `KeyError` for a document not open.
    fn document_memory(&self, doc: &str) -> PyResult<usize> {
        let id = self.doc(doc)?;
        self.inner.session().document_memory(&id).map_err(|e| to_py(&e))
    }

    /// The live view of `doc` (`ooxml_core.Session.view`'s, on the host's documents).
    ///
    /// # Errors
    /// As `Session.view`.
    #[pyo3(signature = (doc, since = None, *, layout, context = None))]
    fn view<'py>(
        &self,
        py: Python<'py>,
        doc: &str,
        since: Option<&str>,
        layout: PyRef<'_, PyCallLayout>,
        context: Option<PyRef<'_, PyCallContext>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        crate::view::view(py, self.inner.session(), (doc, since), &layout, context.as_deref())
    }

    /// A page of the live view as SVG (`ooxml_core.Session.page_svg`'s, on the host's
    /// documents).
    ///
    /// # Errors
    /// As `Session.page_svg`.
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
        crate::view::page_svg(
            py,
            self.inner.session(),
            (doc, page, token),
            (&layout, text),
            context.as_deref(),
        )
    }

    /// Close `doc`: the addresses of its datasets, which stay, frozen.
    ///
    /// # Errors
    /// `KeyError` for a document not open.
    fn close(&self, py: Python<'_>, doc: &str) -> PyResult<Vec<String>> {
        let id = self.doc(doc)?;
        let session = &self.inner;
        py.detach(|| session.close(&id)).map_err(|e| to_py(&e))
    }

    /// `doc`'s bytes at its head and the state to open them again with.
    ///
    /// # Errors
    /// `KeyError` for a document not open; the package's error writing a part.
    fn save(&self, py: Python<'_>, doc: &str) -> PyResult<SavedRow> {
        let id = self.doc(doc)?;
        let session = &self.inner;
        let saved = py.detach(|| session.save(&id)).map_err(|e| to_py(&e))?;
        Ok((
            saved.doc,
            saved.name,
            saved.kind.as_str(),
            saved.version,
            PyBytes::new(py, &saved.data).unbind(),
            saved.state,
        ))
    }

    /// The application's undo (`redo`: redo) of up to `steps` calls on `doc`, with a caller
    /// (`context`, JSON text) its own: `(how many it took back, None)`, or `(None, the
    /// refusal's JSON text)` -- `entangled` when the caller's latest call is not the
    /// document's latest change.
    ///
    /// # Errors
    /// `KeyError` for a document not open.
    #[pyo3(signature = (doc, steps = 1, *, redo = false, context = None))]
    fn history(
        &self,
        py: Python<'_>,
        doc: &str,
        steps: usize,
        redo: bool,
        context: Option<&str>,
    ) -> PyResult<(Option<usize>, Option<String>)> {
        let id = self.doc(doc)?;
        let caller = object_of(context, "context")?;
        let session = &self.inner;
        match py.detach(|| session.history(&id, steps, redo, Some(&caller))) {
            Ok(count) => Ok((Some(count), None)),
            Err(ooxml_session::SessionError::NotOpen { .. }) => {
                Err(PyKeyError::new_err(format!("no open document {doc:?}")))
            }
            Err(error) => Ok((None, Some(compact(&error.to_tool_error().to_json())))),
        }
    }

    /// Run the tool `name` with `arguments` (JSON text) as `roles` (role names; `None`: the
    /// session's) for `context` (JSON text: the caller's), its writes dated `date`
    /// (`2026-11-20T09:30:15Z`, the application's clock; `None`: the system's).
    ///
    /// # Errors
    /// `ValueError` for roles the session does not hold (the application's mistake).
    #[pyo3(signature = (name, arguments, *, roles = None, context = None, date = None))]
    fn call(
        &self,
        py: Python<'_>,
        name: &str,
        arguments: String,
        roles: Option<Vec<String>>,
        context: Option<&str>,
        date: Option<String>,
    ) -> PyResult<Reply> {
        let how = CallAs {
            roles: roles.map(roles_of).transpose()?,
            context: Some(object_of(context, "context")?).filter(|c| !c.is_empty()),
            date,
            ..CallAs::default()
        };
        let session = &self.inner;
        let (text, files, app) = py
            .detach(|| {
                // (the arguments read here, so every step reads them as given; text that is no
                // JSON goes on as text, which the host refuses in its words)
                let given = serde_json::from_str(&arguments).unwrap_or(Value::String(arguments));
                let (mut result, record) = session.call_recorded(name, given, &how)?;
                let files = std::mem::take(&mut result.app.files);
                let app = app_json(&result, &record);
                Ok((
                    compact(&Value::Object(result.to_json())),
                    files,
                    compact(&Value::Object(app)),
                ))
            })
            .map_err(|ooxml_host::NotRun::Roles(why)| PyValueError::new_err(why))?;
        let files = files
            .into_iter()
            .map(|f| (f.name, f.format, PyBytes::new(py, &f.bytes).unbind()))
            .collect();
        Ok((text, files, app))
    }

    /// The files tools saved since the last take, each `(name, format, doc, bytes,
    /// validate, state)`.
    fn take_outputs(&self, py: Python<'_>) -> Vec<OutputRow> {
        self.inner
            .take_outputs()
            .into_iter()
            .map(|o| {
                let state = o.state.as_deref().and_then(|s| s.downcast_ref::<String>()).cloned();
                (
                    o.name,
                    o.format,
                    o.doc,
                    PyBytes::new(py, &o.data).unbind(),
                    o.validate.map(|v| compact(&v)),
                    state,
                )
            })
            .collect()
    }

    /// The roles its calls hold unless narrowed.
    fn roles(&self) -> Vec<&'static str> {
        self.inner.roles().roles().iter().map(|r| r.as_str()).collect()
    }

    /// Its calls hold `roles` (role names) from now on.
    ///
    /// # Errors
    /// `ValueError` for a name that is no role.
    fn set_roles(&self, roles: Vec<String>) -> PyResult<()> {
        self.inner.set_roles(roles_of(roles)?);
        Ok(())
    }

    /// Its checks of numbers read `rules` (JSON text) from now on.
    ///
    /// # Errors
    /// `ValueError` for rules the host cannot take.
    fn set_number_rules(&self, rules: &str) -> PyResult<()> {
        let rules = NumberRules::from_json(&json_of(rules, "rules")?).map_err(PyValueError::new_err)?;
        self.inner.set_number_rules(rules);
        Ok(())
    }

    /// A new round: the round's image budget starts again.
    fn new_round(&self) {
        self.inner.new_round();
    }

    /// Stop every call in flight at its next safe point; later calls stop at once.
    fn stop(&self) {
        self.stop.stop();
    }
}

/// What only the application reads of `result`, with its `record`.
fn app_json(result: &ooxml_tools::ToolResult<Value>, record: &ooxml_host::CallRecord) -> Map<String, Value> {
    let app = &result.app;
    let mut out = Map::new();
    out.insert("touched".into(), Value::from(app.touched.clone()));
    let by: Map<String, Value> = app
        .touched_by_op
        .iter()
        .map(|(op, t)| (op.to_string(), Value::from(t.clone())))
        .collect();
    out.insert("touched_by_op".into(), Value::Object(by));
    out.insert("changed_parts".into(), Value::from(app.changed_parts.clone()));
    let timing = app.timing.as_ref().map(|t| {
        let mut map = Map::new();
        for (key, value) in [
            ("total", Some(t.total)),
            ("lock_wait", Some(t.lock_wait)),
            ("work", Some(t.work)),
            ("held", Some(t.held)),
            ("cpu", t.cpu),
        ] {
            map.insert(key.into(), value.map_or(Value::Null, Value::from));
        }
        Value::Object(map)
    });
    out.insert("timing".into(), timing.unwrap_or(Value::Null));
    out.insert("memory".into(), app.memory.clone().map_or(Value::Null, Value::Object));
    out.insert("budget".into(), Value::from(app.budget));
    out.insert("unchanged".into(), Value::from(app.unchanged));
    out.insert("panic".into(), Value::from(app.panic.clone()));
    out.insert("record".into(), Value::Object(record.to_json()));
    out
}
