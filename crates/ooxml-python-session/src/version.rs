//! Owns: `ooxml_core.Version`, a document at one version, held while Python holds it: its
//! numbers, and what a host keeping a copy beside the session reads from it (the parts and
//! ids changed since another version, its bytes, its records).
//!
//! Must not: change a document, or touch a Python object without the lock.

use crate::session::to_py;
use ooxml_session::Opened;
use ooxml_session_word::prelude::*;
use ooxml_session_word::{BlockRefused, IdStateOf};
use ooxml_tools_word::WordLayout;
use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use serde_json::Value;

/// A document at one version (`ooxml_core.Version`), held while Python holds it: a call
/// given it (`CallDocuments(at=[version])` in its `CallOptions`) reads that version,
/// whatever was committed since.
#[pyclass(frozen, name = "Version", module = "ooxml_core")]
pub struct PyVersion {
    pub(crate) inner: Opened,
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

    /// The state's number, as the tool layer numbers the document's states: a step a new
    /// one, an undo or redo the number of the state it brings back (numbers repeat).
    #[getter]
    fn state(&self) -> u64 {
        self.inner.number()
    }

    fn __repr__(&self) -> String {
        format!("<ooxml_core.Version {} {}>", self.inner.id(), self.inner.version())
    }

    /// The names of the parts whose content differs from `before` (a version of the same
    /// document), none written.
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

    /// The records of placed values this version keeps (JSON text, `Opened::records`): the
    /// tables' and charts' records, the entries left out, the paragraphs' text facts and
    /// labels; `None` for a document that is not a Word document.
    fn records(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let version = &self.inner;
        let found = py.detach(|| version.records()).map_err(|e| to_py(&e))?;
        Ok(found.map(|v| v.to_string()))
    }

    /// The version's validation problems, each ``(code, part, detail)`` (docx-agent's
    /// ``validate.Problem``); ``None`` for a document that is not a Word document.
    fn problems(&self, py: Python<'_>) -> PyResult<Option<Vec<(String, String, String)>>> {
        let version = &self.inner;
        let found = py.detach(|| version.problems()).map_err(|e| to_py(&e))?;
        Ok(found.map(|list| list.into_iter().map(|p| (p.code, p.part, p.detail)).collect()))
    }

    /// The Word file kind of the version's package (``docx``, ``docm``, ``dotx``, ``dotm``:
    /// its main part's content type); ``None`` for a document that is not a Word document.
    fn file_kind(&self, py: Python<'_>) -> PyResult<Option<&'static str>> {
        let version = &self.inner;
        py.detach(|| version.file_kind()).map_err(|e| to_py(&e))
    }

    /// What ``open_document`` says of the version (JSON text: ``title``, ``paragraphs``,
    /// ``tables``, ``headings``, ``comments``, ``revisions``, ``template``); ``None`` for a
    /// document that is not a Word document.
    fn summary(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let version = &self.inner;
        let found = py
            .detach(|| version.summary())
            .map_err(|e| PyValueError::new_err(e.message().to_owned()))?;
        Ok(found.map(|v| v.to_string()))
    }

    /// The version's id state (``Document.id_state``'s string), the version numbered
    /// ``number`` of ``last`` by the host, with the retired ids as the host numbers them:
    /// ``gone`` and ``drawings_gone`` (``{value: number}``), ``earlier`` (``{number: words or
    /// None}``, the changes before the document was saved) and ``said`` (``{number: words}``,
    /// who made the others); ``None`` for a document that is not a Word document.
    #[pyo3(signature = (number, last, *, gone, drawings_gone, earlier, said))]
    #[allow(clippy::too_many_arguments)]
    fn id_state(
        &self,
        py: Python<'_>,
        number: i64,
        last: i64,
        gone: Vec<(u32, i64)>,
        drawings_gone: Vec<(u64, i64)>,
        earlier: Vec<(i64, Option<String>)>,
        said: Vec<(i64, String)>,
    ) -> PyResult<Option<String>> {
        let host = IdStateOf {
            version: number,
            last,
            gone,
            drawings_gone,
            earlier,
            said,
        };
        let version = &self.inner;
        py.detach(|| version.id_state(&host)).map_err(|e| to_py(&e))
    }

    /// What kind of block ``id`` names in the version: ``"paragraph"``, ``"table"`` or
    /// ``"content control"`` (a block-level one), a renamed id followed; ``None`` when it
    /// names none of its blocks (or the document is not a Word document).
    fn block_kind(&self, py: Python<'_>, id: &str) -> PyResult<Option<&'static str>> {
        let version = &self.inner;
        py.detach(|| version.block_kind(id)).map_err(|e| to_py(&e))
    }

    /// The id of the body's top-level block holding ``id`` (any id the tools take: a
    /// nested paragraph, a range, a row or cell, a drawing, a note's or comment's
    /// reference, a bookmark, a section's break), a renamed id followed; ``None`` for what
    /// is not in the body. Raises ``KeyError`` for an id the version lacks, ``ValueError``
    /// for a ref or a span.
    fn block_of(&self, py: Python<'_>, id: &str) -> PyResult<Option<String>> {
        let version = &self.inner;
        let found = py.detach(|| version.block_of(id)).map_err(|e| to_py(&e))?;
        found.map_err(refused)
    }

    /// The ids of the body's top-level blocks from the one holding ``first`` through the
    /// one holding ``last`` (``block_of``), in order. Raises ``KeyError`` for an id the
    /// version lacks, ``ValueError`` for an end outside the body or a span the wrong way
    /// round.
    fn blocks_between(&self, py: Python<'_>, first: &str, last: &str) -> PyResult<Vec<String>> {
        let version = &self.inner;
        let found = py
            .detach(|| version.blocks_between(first, last))
            .map_err(|e| to_py(&e))?;
        found.map_err(refused)
    }

    /// Whether the version has the drawing ``id`` (``d:3``, ``d:3#1``).
    fn has_drawing(&self, py: Python<'_>, id: &str) -> PyResult<bool> {
        let version = &self.inner;
        py.detach(|| version.has_drawing(id)).map_err(|e| to_py(&e))
    }

    /// The texts a check of numbers reads in the version (JSON text: ``{"texts": [{"at",
    /// "text", "written", "page"}], "warnings"}``, ``{"error": ...}`` for a refusal, ``null``
    /// for a document that is not a Word document). ``request`` (JSON): ``range`` (a check's,
    /// refs resolved) and ``pages``; ``doc``: the document as the host names it; ``pages``:
    /// the engine its texts are placed with (``Session.pages``; ``None``: not placed).
    #[pyo3(signature = (doc, request, pages = None))]
    fn number_texts(
        &self,
        py: Python<'_>,
        doc: &str,
        request: &str,
        pages: Option<PyRef<'_, crate::PyWordPages>>,
    ) -> String {
        let layout = pages.map(|p| p.port());
        let version = &self.inner;
        py.detach(|| crate::values::number_texts(version, doc, request, layout.as_ref().map(|l| l as &dyn WordLayout)))
    }

    /// The values a scale's targets hold in the version, each with the step it is styled as
    /// now (JSON text: ``{"values": [...]}``, as ``number_texts`` answers). ``request``
    /// (JSON): ``scale`` (``name``, ``steps`` with ``fill``, ``text`` and ``label``,
    /// ``targets``, ``labels``), ``range`` and ``pages``.
    #[pyo3(signature = (doc, request, pages = None))]
    fn scale_values(
        &self,
        py: Python<'_>,
        doc: &str,
        request: &str,
        pages: Option<PyRef<'_, crate::PyWordPages>>,
    ) -> String {
        let layout = pages.map(|p| p.port());
        let version = &self.inner;
        py.detach(|| crate::values::scale_values(version, doc, request, layout.as_ref().map(|l| l as &dyn WordLayout)))
    }

    /// What changed since `before`, as docx-agent's ``Result.touched`` entries (JSON text).
    fn touched_since(&self, py: Python<'_>, before: PyRef<'_, PyVersion>) -> PyResult<String> {
        let (now, then) = (&self.inner, &before.inner);
        let found = py.detach(|| now.touched_since(then)).map_err(|e| to_py(&e))?;
        Ok(Value::Array(found).to_string())
    }
}

/// A block refused as Python says it: an id the version lacks a `KeyError`, else a
/// `ValueError`.
fn refused(why: BlockRefused) -> PyErr {
    match why {
        BlockRefused::NotFound(message) => PyKeyError::new_err(message),
        BlockRefused::Refused(message) => PyValueError::new_err(message),
    }
}
