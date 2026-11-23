//! Owns: `ooxml_core.CallOptions`, everything a host gives one `Session.call` or
//! `Session.edit` beside the tool's name and arguments -- who calls (a `CallContext`), the
//! reply's limit, the other open documents it reads (`ooxml_core.CallDocuments`: their
//! versions and the host's names for them), the document's layout (`ooxml_core.CallLayout`:
//! the page engine, the raster pages are drawn with, the placer of blocks), the blobs, the
//! values written by reference -- read into a [`CallContext`] once, while the
//! interpreter lock is held; and the adapter of a Python placer ([`PyPlacer`]: a `Placer` called with the
//! lock taken back). A new kind of per-call input is one more keyword here, not one more
//! argument of every call.
//!
//! Must not: run a call, or touch a Python object without the lock.

use crate::adapters::PyBlobs;
use crate::context::PyCallContext;
use crate::layout::PyWordPages;
use crate::locale::locale_of;
use crate::raster::PyPageRaster;
use crate::version::PyVersion;
use ooxml_session::{CallContext, DocId, DocNames, Opened};
use ooxml_tools::{BudgetSpan, ImageBudget};
use ooxml_tools_word::{Blobs, HostTable, HostTables, LayoutCounts, Placer, Sources, Written};
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
/// refused for a table not in its document says it is one of them. `sources`: the rows of
/// the sources the call names, as the host resolved them before the call, as JSON text (a
/// list, each `{"given", "address", "columns", "types", "rows", "sources", "doc", "name",
/// "version", "stale", "cache"}`: ooxml-edit's `Rows`, keyed by the value as the call gives
/// it; a value `null`, a logical value, a number, text, `{"date": [y, m, d]}`, `{"datetime":
/// [y, m, d, h, min, s, us]}`, `{"time": [h, min, s, us]}`, `{"int": "digits"}` or
/// `{"other": kind}`), or the host's refusal of one (`{"given", "refused": {"message",
/// "options"}}`, an unknown dataset, which the tool words with what its document says of the
/// value). One may serve many calls.
#[pyclass(frozen, name = "CallDocuments", module = "ooxml_core")]
pub struct PyCallDocuments {
    at: Vec<Opened>,
    names: DocNames,
    tables: Option<Arc<dyn HostTables>>,
    sources: Sources,
}

#[pymethods]
impl PyCallDocuments {
    /// # Errors
    /// `ValueError` for `sources` that is not such a list.
    #[new]
    #[pyo3(signature = (*, at = None, names = None, tables = None, sources = None))]
    fn new(
        at: Option<Vec<PyRef<'_, PyVersion>>>,
        names: Option<BTreeMap<String, String>>,
        tables: Option<Py<PyAny>>,
        sources: Option<&str>,
    ) -> PyResult<Self> {
        let mut known = DocNames::new();
        for (name, id) in names.into_iter().flatten() {
            if let Some(id) = DocId::parse(&id) {
                known.insert(name, id);
            }
        }
        let sources = match sources {
            None => Sources::default(),
            Some(given) => serde_json::from_str::<Value>(given)
                .ok()
                .as_ref()
                .and_then(Sources::from_json)
                .ok_or_else(|| PyValueError::new_err("sources is a JSON list of the rows the call's sources name"))?,
        };
        Ok(Self {
            at: at.iter().flatten().map(|v| v.inner.clone()).collect(),
            names: known,
            tables: tables.map(|t| Arc::new(PyHostTables(t)) as Arc<dyn HostTables>),
            sources,
        })
    }
}

/// The settings that decide what a call writes (`ooxml_core.CallWriting`, `CallContext`'s
/// `writing`). `date`: the moment its writes are dated by (a revision's, a comment's), UTC to
/// the second (`2026-11-20T09:30:15Z`) -- the host's clock, as docx-agent dates them; `None`:
/// the session's clock. `typography`: whether the text it writes is spaced by its language
/// (docx-agent's `docx_format(typography=...)`; `False`: as given). `layout_counts`:
/// `(pages, lines)` of the host's layout of the version the call reads, the statistics a
/// save writes into `docProps/app.xml` (`save_document`); `None`: not laid out. `locale`:
/// the locale of the table of contents' style names and the separator between them the call
/// writes (ooxml-common's `Locale` or a BCP 47 tag, docx-agent's document locale); `None`:
/// the session's, else each document's own language. One may serve many calls.
#[pyclass(frozen, name = "CallWriting", module = "ooxml_core")]
pub struct PyCallWriting {
    pub(crate) date: Option<String>,
    pub(crate) as_given: bool,
    pub(crate) counts: Option<LayoutCounts>,
    pub(crate) locale: Option<ooxml_numfmt::Locale>,
}

#[pymethods]
impl PyCallWriting {
    /// # Errors
    /// `ValueError` for a `date` not of that form, or a locale refused.
    #[new]
    #[pyo3(signature = (*, date = None, typography = true, layout_counts = None, locale = None))]
    fn new(
        date: Option<&str>,
        typography: bool,
        layout_counts: Option<(u64, u64)>,
        locale: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        if date.is_some_and(|d| !is_utc_second(d)) {
            return Err(PyValueError::new_err("date is UTC to the second: 2026-11-20T09:30:15Z"));
        }
        Ok(Self {
            date: date.map(str::to_owned),
            as_given: !typography,
            counts: layout_counts.map(|(pages, lines)| LayoutCounts { pages, lines }),
            locale: locale.map(locale_of).transpose()?,
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

impl PyCallLayout {
    /// Its page engine, which the live view lays the document out with.
    ///
    /// # Errors
    /// `ValueError` where it has none.
    pub(crate) fn word_layout(&self) -> PyResult<Arc<dyn ooxml_tools_word::WordLayout>> {
        self.pages.clone().ok_or_else(|| {
            PyValueError::new_err("the live view lays the document out with a CallLayout(pages=WordPages)")
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
            built.tables.clone_from(&documents.tables);
            built.sources.clone_from(&documents.sources);
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
