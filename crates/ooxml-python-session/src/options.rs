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
use ooxml_tools_word::{Blobs, Placer, Written};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// What a call needs about the open documents it names (`ooxml_core.CallDocuments`).
/// `at`: the versions it reads (`Version`s; a document not among them is read at its head).
/// `names`: the host's names for the documents (`{name: the session's id}`): the arguments
/// name documents by these, and the reply names them so; a name whose id is not one of the
/// session's names nothing (the call then refuses it as not open). One may serve many calls.
#[pyclass(frozen, name = "CallDocuments", module = "ooxml_core")]
pub struct PyCallDocuments {
    at: Vec<Opened>,
    names: DocNames,
}

#[pymethods]
impl PyCallDocuments {
    #[new]
    #[pyo3(signature = (*, at = None, names = None))]
    fn new(at: Option<Vec<PyRef<'_, PyVersion>>>, names: Option<BTreeMap<String, String>>) -> Self {
        let mut known = DocNames::new();
        for (name, id) in names.into_iter().flatten() {
            if let Some(id) = DocId::parse(&id) {
                known.insert(name, id);
            }
        }
        Self {
            at: at.iter().flatten().map(|v| v.inner.clone()).collect(),
            names: known,
        }
    }
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
        }
        built.placer = placer.map(|p| Arc::new(PyPlacer(p)) as Arc<dyn Placer>);
        built.layout = layout.map(|pages| Arc::new(pages.port()) as Arc<dyn ooxml_word_edit::FieldLayout>);
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
