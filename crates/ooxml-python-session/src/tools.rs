//! Owns: `ooxml_core.tool_specs`, the registry's export for a document kind as a session
//! made now would serve it: the tools' definitions; and
//! `ooxml_core.new_document`, a new blank document's bytes, which the host opens.
//!
//! Must not: dispatch a call.

use crate::errors::kind_of;
use ooxml_tools::compact;
use ooxml_tools_word::new_document::Blank;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use serde_json::Value;

/// The tools' definitions for `kind` as JSON text (a list of `ToolSpec` exports): what a
/// session made now would serve.
#[pyfunction]
pub fn tool_specs(py: Python<'_>, kind: &str) -> PyResult<String> {
    let kind = kind_of(kind)?;
    Ok(py.detach(|| compact(&Value::Array(ooxml_session_word::registry().export(kind)))))
}

/// A new blank document's package (`ooxml_tools_word::new_document::Blank`), as bytes:
/// `page`, its size (A4, Letter, Legal, A5, A3) and whether it is landscape; the text
/// `language`; `separators`, the regional settings' decimal symbol and list separator;
/// `properties`, the title and author (`None`: none) and when it was made (W3CDTF,
/// `2026-11-21T09:00:00Z`).
///
/// # Errors
/// `ValueError` for a page size not named.
#[pyfunction]
#[pyo3(signature = (*, page, language, separators, properties))]
pub fn new_document<'py>(
    py: Python<'py>,
    page: (String, bool),
    language: String,
    separators: (String, String),
    properties: (Option<String>, Option<String>, String),
) -> PyResult<Bound<'py, PyBytes>> {
    let (title, author, created) = properties;
    let blank = Blank {
        page: page.0,
        landscape: page.1,
        language,
        separators,
        title,
        author,
        created,
    };
    let bytes = py.detach(|| blank.bytes()).map_err(PyValueError::new_err)?;
    Ok(PyBytes::new(py, &bytes))
}
