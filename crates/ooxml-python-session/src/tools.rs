//! Owns: `ooxml_core.tool_specs` and `ooxml_core.tool_hosting`, the registry's exports for a
//! document kind as a session made now would serve them: the tools' definitions, and how a
//! host hands each tool's calls to the session (each Rust tool's `Hosting`).
//!
//! Must not: dispatch a call.

use crate::session::kind_of;
use ooxml_session::Registry;
use ooxml_tools::compact;
use pyo3::prelude::*;
use serde_json::Value;

/// The tools' definitions for `kind` as JSON text (a list of `ToolSpec` exports): what a
/// session made now would serve.
#[pyfunction]
pub fn tool_specs(py: Python<'_>, kind: &str) -> PyResult<String> {
    let kind = kind_of(kind)?;
    Ok(py.detach(|| compact(&Value::Array(Registry::standard().export(kind)))))
}

/// How a host hands each tool for `kind` its calls, as JSON text: `{name: {"served",
/// "blobs", "layout", "keeps_written"}}`, each a boolean (`ooxml_tools_word::Hosting`).
#[pyfunction]
pub fn tool_hosting(py: Python<'_>, kind: &str) -> PyResult<String> {
    let kind = kind_of(kind)?;
    Ok(py.detach(|| compact(&Value::Object(Registry::standard().hosting(kind)))))
}
