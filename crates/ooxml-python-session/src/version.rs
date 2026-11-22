//! Owns: `ooxml_core.Version`, a document at one version, held while Python holds it: its
//! numbers, and what a host keeping a copy beside the session reads from it (the parts and
//! ids changed since another version, its bytes, its records).
//!
//! Must not: change a document, or touch a Python object without the lock.

use crate::session::to_py;
use ooxml_session::{IdStateOf, Opened};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
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

    /// What changed since `before`, as docx-agent's ``Result.touched`` entries (JSON text).
    fn touched_since(&self, py: Python<'_>, before: PyRef<'_, PyVersion>) -> PyResult<String> {
        let (now, then) = (&self.inner, &before.inner);
        let found = py.detach(|| now.touched_since(then)).map_err(|e| to_py(&e))?;
        Ok(Value::Array(found).to_string())
    }

    /// The ids this version gave that `before` had not, and its renames that `before`
    /// lacks: ``(values, {old id: id now})`` -- what a view of the document kept beside the
    /// session takes to give no id twice and follow renamed ids, without reading a part.
    fn ids_since(&self, py: Python<'_>, before: PyRef<'_, PyVersion>) -> PyResult<IdsSince> {
        let (now, then) = (&self.inner, &before.inner);
        let found = py.detach(|| now.ids_since(then)).map_err(|e| to_py(&e))?;
        Ok((found.given, found.renamed))
    }
}

/// What `Version.ids_since` answers: the id values given, and the renames (old id, id now).
type IdsSince = (Vec<u32>, Vec<(String, String)>);
