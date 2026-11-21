//! Owns: `ooxml_core.CallOptions`, everything a host gives one `Session.call` or
//! `Session.edit` beside the tool's name and arguments -- who calls (a `CallContext`), the
//! reply's limit, the other open documents it reads (`ooxml_core.CallDocuments`: their
//! versions and the host's names for them), the placer of blocks, the page engine, the
//! blobs, the values written by reference -- read into a [`CallContext`] once, while the
//! interpreter lock is held; and the adapter of a Python placer ([`PyPlacer`]: a `Placer` called with the
//! lock taken back). A new kind of per-call input is one more keyword here, not one more
//! argument of every call.
//!
//! Must not: run a call, or touch a Python object without the lock.

use crate::adapters::PyBlobs;
use crate::layout::PyWordPages;
use crate::session::{PyCallContext, PyVersion};
use ooxml_session::{CallContext, DocId, DocNames, HostIds, Opened};
use ooxml_tools_word::{Blobs, HostTable, HostTables, LayoutCounts, Placer, Written};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// What a call needs about the open documents it names (`ooxml_core.CallDocuments`).
/// `at`: the versions it reads (`Version`s; a document not among them is read at its head).
/// `names`: the host's names for the documents (`{name: the session's id}`): the arguments
/// name documents by these, and the reply names them so; a name whose id is not one of the
/// session's names nothing (the call then refuses it as not open). `tables`: the tables of
/// the open documents the host holds itself, a callable answering a list of `(name, doc,
/// kind)` -- as a `from` names it (`t:stock`, `d2/t:stock`), the host's name of its
/// document, its format (`xlsx`) -- called with the lock taken back, only where a call
/// refused for a table not in its document says it is one of them. One may serve many
/// calls.
#[pyclass(frozen, name = "CallDocuments", module = "ooxml_core")]
pub struct PyCallDocuments {
    at: Vec<Opened>,
    names: DocNames,
    tables: Option<Arc<dyn HostTables>>,
}

#[pymethods]
impl PyCallDocuments {
    #[new]
    #[pyo3(signature = (*, at = None, names = None, tables = None))]
    fn new(
        at: Option<Vec<PyRef<'_, PyVersion>>>,
        names: Option<BTreeMap<String, String>>,
        tables: Option<Py<PyAny>>,
    ) -> Self {
        let mut known = DocNames::new();
        for (name, id) in names.into_iter().flatten() {
            if let Some(id) = DocId::parse(&id) {
                known.insert(name, id);
            }
        }
        Self {
            at: at.iter().flatten().map(|v| v.inner.clone()).collect(),
            names: known,
            tables: tables.map(|t| Arc::new(PyHostTables(t)) as Arc<dyn HostTables>),
        }
    }
}

/// How a call writes (`ooxml_core.CallWriting`, `CallContext`'s `writing`). `date`: the
/// moment its writes are dated by (a revision's, a comment's), UTC to the second
/// (`2026-11-20T09:30:15Z`) -- the host's clock, as docx-agent dates them; `None`: the
/// session's clock. `typography`: whether the text it writes is spaced by its language
/// (docx-agent's `docx_format(typography=...)`; `False`: as given). `layout_counts`:
/// `(pages, lines)` of the host's layout of the version the call reads, where it has laid it
/// out -- the statistics a save writes into `app.xml` (`save_document`); `None`: not laid
/// out. One may serve many calls.
#[pyclass(frozen, name = "CallWriting", module = "ooxml_core")]
pub struct PyCallWriting {
    pub(crate) date: Option<String>,
    pub(crate) as_given: bool,
    pub(crate) counts: Option<LayoutCounts>,
}

#[pymethods]
impl PyCallWriting {
    /// # Errors
    /// `ValueError` for a `date` not of that form.
    #[new]
    #[pyo3(signature = (*, date = None, typography = true, layout_counts = None))]
    fn new(date: Option<&str>, typography: bool, layout_counts: Option<(u64, u64)>) -> PyResult<Self> {
        if date.is_some_and(|d| !is_utc_second(d)) {
            return Err(PyValueError::new_err("date is UTC to the second: 2026-11-20T09:30:15Z"));
        }
        Ok(Self {
            date: date.map(str::to_owned),
            as_given: !typography,
            counts: layout_counts.map(|(pages, lines)| LayoutCounts { pages, lines }),
        })
    }
}

/// Whether `date` is a moment as a host dates writes: `YYYY-MM-DDTHH:MM:SSZ`.
fn is_utc_second(date: &str) -> bool {
    let b = date.as_bytes();
    b.len() == 20
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            19 => *c == b'Z',
            _ => c.is_ascii_digit(),
        })
}

/// What the host's id ledger of a Word document holds beside the parts it bridges
/// (`ooxml_core.HostIds`, `Session.bridge`'s `ids`): `drawn`, where its tool's draws of new
/// ids got to in each seed's sequence (`{seed: index}`, docx-agent's id pool cursors: an id
/// it drew and did not write is not drawn again); `aliases`, its renames (`{old: new}`: a
/// block it stamped is found by its old id too).
#[pyclass(frozen, name = "HostIds", module = "ooxml_core")]
pub struct PyHostIds {
    pub(crate) inner: HostIds,
}

#[pymethods]
impl PyHostIds {
    #[new]
    #[pyo3(signature = (*, drawn = None, aliases = None))]
    fn new(drawn: Option<BTreeMap<String, usize>>, aliases: Option<BTreeMap<String, String>>) -> Self {
        Self {
            inner: HostIds {
                drawn: drawn.unwrap_or_default().into_iter().collect(),
                aliases: aliases.unwrap_or_default().into_iter().collect(),
            },
        }
    }
}

/// What one call runs with beside its name and arguments (`ooxml_core.CallOptions`).
/// `context`: who calls, a token that stops it, how long it may run (a `CallContext`).
/// `limit`: the reply within so many characters, as the model is sent it (`call` only).
/// `documents`: what the call needs about the open documents it names (a `CallDocuments`:
/// the versions it reads and the host's names for them). `placer`: where it may ask where blocks are laid out -- called with the blocks
/// asked as JSON text (a list, each an id then the ids it joins), it answers each one's
/// `placements` or `unknown` as JSON text (a list of objects), or `None` where it cannot.
/// `layout`: the document's page engine (a `WordPages`, the host's kept one), which an edit
/// whose results come from the pages (`word_fields`, a header's page number,
/// `word_template`'s reflow) lays its versions out with; `None`: such a result is left to the
/// host. `blobs`: the host's blobs, an object with `blob(handle)` (`(name, mime, bytes)` or
/// `None`) and `listed()` (`[(handle, name)]`), asked with the lock taken back
/// (`word_drawings` insert_picture). `written`: the values the host wrote into the arguments
/// for references, as JSON text (`[{"field", "start", "end", "text", "record", "brief"}]`,
/// ooxml-edit's `call.facts`, each with its own `"markdown"` where it has one), which an
/// editing call keeps on the paragraphs they went into. One may serve many calls.
#[pyclass(frozen, name = "CallOptions", module = "ooxml_core")]
pub struct PyCallOptions {
    pub(crate) context: CallContext,
    pub(crate) limit: Option<usize>,
}

#[pymethods]
impl PyCallOptions {
    /// # Errors
    /// `ValueError` for `written` that is not such a list.
    #[new]
    #[pyo3(signature = (*, context = None, limit = None, documents = None, placer = None, layout = None,
                        blobs = None, written = None))]
    fn new(
        context: Option<PyRef<'_, PyCallContext>>,
        limit: Option<usize>,
        documents: Option<PyRef<'_, PyCallDocuments>>,
        placer: Option<Py<PyAny>>,
        layout: Option<PyRef<'_, PyWordPages>>,
        blobs: Option<Py<PyAny>>,
        written: Option<&str>,
    ) -> PyResult<Self> {
        let mut built = context.map(|c| c.inner.clone()).unwrap_or_default();
        if let Some(documents) = documents {
            built.at.clone_from(&documents.at);
            built.names = documents.names.clone();
            built.tables.clone_from(&documents.tables);
        }
        built.placer = placer.map(|p| Arc::new(PyPlacer(p)) as Arc<dyn Placer>);
        built.layout = layout.map(|pages| Arc::new(pages.port()) as Arc<dyn ooxml_tools_word::WordLayout>);
        built.blobs = blobs.map(|b| Arc::new(PyBlobs(b)) as Arc<dyn Blobs>);
        if let Some(written) = written {
            built.written = serde_json::from_str::<Value>(written)
                .ok()
                .as_ref()
                .and_then(Written::list_from_json)
                .ok_or_else(|| PyValueError::new_err("written is a JSON list of the values written by reference"))?;
        }
        Ok(Self { context: built, limit })
    }
}

/// The call context of `options` (the default's where there are none).
pub(crate) fn context_of(options: Option<&PyCallOptions>) -> CallContext {
    options.map(|o| o.context.clone()).unwrap_or_default()
}

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

/// Python's tables of the documents the host holds (`tables() -> [(name, doc, kind)]`),
/// called with the interpreter lock taken back; none where it fails.
struct PyHostTables(Py<PyAny>);

impl HostTables for PyHostTables {
    fn tables(&self) -> Vec<HostTable> {
        let found = Python::attach(|py| {
            let answer = self.0.call0(py).ok()?;
            answer.extract::<Vec<(String, String, String)>>(py).ok()
        });
        found
            .into_iter()
            .flatten()
            .map(|(name, doc, kind)| HostTable { name, doc, kind })
            .collect()
    }
}
