//! Owns: `ooxml_core.CallOptions`, everything a host gives one `Session.call` or
//! `Session.edit` beside the tool's name and arguments -- who calls (a `CallContext`), the
//! reply's limit, the other open documents it reads (`ooxml_core.CallDocuments`: their
//! versions and the host's names for them), the document's layout (`ooxml_core.CallLayout`:
//! the page engine, the raster pages are drawn with, the placer of blocks), the blobs, the
//! values written by reference -- read into a [`CallContext`] once, while the
//! interpreter lock is held; `ooxml_core.Limits`, the host's limits on a call's reply that a
//! `CallContext` carries ([`PyLimits`]); and the adapter of a Python placer ([`PyPlacer`]: a `Placer` called with the
//! lock taken back). A new kind of per-call input is one more keyword here, not one more
//! argument of every call.
//!
//! Must not: run a call, or touch a Python object without the lock.

use crate::adapters::PyBlobs;
use crate::layout::PyWordPages;
use crate::raster::PyPageRaster;
use crate::session::{PyCallContext, PyVersion};
use ooxml_session::{CallContext, DocId, DocNames, Opened};
use ooxml_tools::{BudgetSpan, ImageBudget};
use ooxml_tools_word::{Blobs, Placer, Written};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The host's limits on a call's reply (`ooxml_core.Limits`, docx-agent's `Limits` of the
/// same names): `max_result_chars` (a page's characters; 24,000 unless given),
/// `max_list_items` (a listing's items per page; 50), `max_images_per_call` (4) and
/// `max_image_edge` (an image's longest edge, pixels; 2,576). A `CallContext` carries them.
#[pyclass(frozen, name = "Limits", module = "ooxml_core")]
pub struct PyLimits {
    pub(crate) inner: ooxml_tools::Limits,
}

#[pymethods]
impl PyLimits {
    /// # Errors
    /// `ValueError` for a limit of 0.
    #[new]
    #[pyo3(signature = (*, max_result_chars = None, max_list_items = None, max_images_per_call = None,
                        max_image_edge = None))]
    fn new(
        max_result_chars: Option<usize>,
        max_list_items: Option<usize>,
        max_images_per_call: Option<usize>,
        max_image_edge: Option<u32>,
    ) -> PyResult<Self> {
        let mut inner = ooxml_tools::Limits::DEFAULT;
        for (given, kept, name) in [
            (max_result_chars, &mut inner.result_chars, "max_result_chars"),
            (max_list_items, &mut inner.list_items, "max_list_items"),
            (max_images_per_call, &mut inner.images_per_call, "max_images_per_call"),
        ] {
            match given {
                Some(0) => return Err(PyValueError::new_err(format!("{name} is 1 or more"))),
                Some(n) => *kept = n,
                None => {}
            }
        }
        match max_image_edge {
            Some(0) => return Err(PyValueError::new_err("max_image_edge is 1 or more")),
            Some(edge) => inner.image_edge = i64::from(edge),
            None => {}
        }
        Ok(Self { inner })
    }
}

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

/// What a call reads of the document's layout (`ooxml_core.CallLayout`). `pages`: the
/// document's page engine (a `WordPages`, the host's kept one), which an edit whose results
/// come from the pages (`word_fields`, a header's page number, `word_template`'s reflow)
/// lays its versions out with, and `check` and `render` read the pages of. `raster`: the
/// faces pages are drawn with (a `PageRaster` of the system's fonts and the font folders),
/// from which `render` draws each document's pages; `None`: `render` is left to the host.
/// `image_budgets`: the host's budgets of images kept across calls, as they stand when the
/// call starts -- `(name, limit, used)`, `name` `"session"` or `"round"` -- which `render`
/// refuses past, worded as the host words it (the call's own limit is the context's
/// `Limits`).
/// `placer`: where the call may ask where blocks are laid out -- called with the blocks
/// asked as JSON text (a list, each an id then the ids it joins), it answers each one's
/// `placements` or `unknown` as JSON text (a list of objects), or `None` where it cannot.
/// One may serve many calls.
#[pyclass(frozen, name = "CallLayout", module = "ooxml_core")]
pub struct PyCallLayout {
    pages: Option<Arc<dyn ooxml_tools_word::WordLayout>>,
    raster: Option<Arc<ooxml_tools_word::PageRaster>>,
    image_budgets: Vec<ImageBudget>,
    placer: Option<Arc<dyn Placer>>,
}

#[pymethods]
impl PyCallLayout {
    /// # Errors
    /// `ValueError` for a budget not named `"session"` or `"round"`.
    #[new]
    #[pyo3(signature = (pages = None, *, raster = None, image_budgets = None, placer = None))]
    fn new(
        pages: Option<PyRef<'_, PyWordPages>>,
        raster: Option<PyRef<'_, PyPageRaster>>,
        image_budgets: Option<Vec<(String, usize, usize)>>,
        placer: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let image_budgets = image_budgets
            .into_iter()
            .flatten()
            .map(|(name, limit, used)| {
                BudgetSpan::parse(&name)
                    .map(|span| ImageBudget { span, limit, used })
                    .ok_or_else(|| PyValueError::new_err(format!("image budget {name:?}: \"session\" or \"round\"")))
            })
            .collect::<PyResult<_>>()?;
        Ok(Self {
            pages: pages.map(|p| Arc::new(p.port()) as Arc<dyn ooxml_tools_word::WordLayout>),
            raster: raster.map(|r| Arc::clone(&r.inner)),
            image_budgets,
            placer: placer.map(|p| Arc::new(PyPlacer(p)) as Arc<dyn Placer>),
        })
    }
}

/// What one call runs with beside its name and arguments (`ooxml_core.CallOptions`).
/// `context`: who calls, a token that stops it, how long it may run (a `CallContext`).
/// `limit`: the reply within so many characters, as the model is sent it (`call` only).
/// `documents`: what the call needs about the open documents it names (a `CallDocuments`:
/// the versions it reads and the host's names for them). `layout`: what it reads of the
/// document's layout (a `CallLayout`: the page engine, the raster, the placer); `None`:
/// what needs them is left to the host. `blobs`: the host's blobs, an object with `blob(handle)` (`(name, mime, bytes)` or
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
    #[pyo3(signature = (*, context = None, limit = None, documents = None, layout = None, blobs = None,
                        written = None))]
    fn new(
        context: Option<PyRef<'_, PyCallContext>>,
        limit: Option<usize>,
        documents: Option<PyRef<'_, PyCallDocuments>>,
        layout: Option<PyRef<'_, PyCallLayout>>,
        blobs: Option<Py<PyAny>>,
        written: Option<&str>,
    ) -> PyResult<Self> {
        let mut built = context.map(|c| c.inner.clone()).unwrap_or_default();
        if let Some(documents) = documents {
            built.at.clone_from(&documents.at);
            built.names = documents.names.clone();
        }
        if let Some(layout) = layout {
            built.placer.clone_from(&layout.placer);
            built.layout.clone_from(&layout.pages);
            built.raster.clone_from(&layout.raster);
            built.image_budgets.clone_from(&layout.image_budgets);
        }
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
