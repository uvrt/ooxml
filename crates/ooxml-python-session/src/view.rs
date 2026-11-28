//! Owns: the live view's Python side (`HostSession.view`, `HostSession.page_svg`; an
//! application's API, not a tool), on the host's documents: laid out by each document's own
//! page engine, within the host's limits; a view as a dict, a page as `(svg, warnings)`, every
//! Rust call made with the interpreter lock released, and the mapping of the view's errors --
//! `ooxml_core.UnknownToken` (a `LookupError`), `IndexError` for a page the version does not
//! have, `ValueError` for a text mode there is none of, `ooxml_core.ViewRefused` (args: the
//! refusal's code, its message, its error as JSON text) for a version or a mode not drawn yet,
//! `TimeoutError` past the host's limit on laying out.
//!
//! The changes feed's Python side too (`HostSession.changes`): the host's answer as JSON
//! text, the pictures new since `since` beside it as `(hash, mime, bytes)`, the call made with
//! the interpreter lock released (readable from another thread while agents edit); a
//! refusal raised as `ViewRefused` (`not_supported` for a format with no feed), `TimeoutError`
//! past the host's limit on laying out.
//!
//! Must not: do the view's work (ooxml-session does), or hold a Python object while the
//! version is laid out or painted.

use crate::errors::to_py;
use ooxml_host::HostSession;
use ooxml_session::{DocId, SessionError};
use ooxml_session_word::prelude::*;
use ooxml_session_word::{LiveView, TextMode, View, ViewError};
use ooxml_tools::{ErrorCode, Failure};
use pyo3::exceptions::{PyIndexError, PyLookupError, PyRuntimeError, PyTimeoutError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

pyo3::create_exception!(
    ooxml_core,
    UnknownToken,
    PyLookupError,
    "A token the live view was given that names no version kept for the document."
);

pyo3::create_exception!(
    ooxml_core,
    ViewRefused,
    pyo3::exceptions::PyException,
    "The live view does not draw this yet: args (code, message, error as JSON text)."
);

/// The live view's error as Python raises it.
fn raised(error: &ViewError) -> PyErr {
    let message = error.to_string();
    match error {
        ViewError::Session(error) => to_py(error),
        ViewError::NotWord { .. } | ViewError::Mode(_) => PyValueError::new_err(message),
        ViewError::UnknownToken { .. } => UnknownToken::new_err(message),
        ViewError::NoPage { .. } => PyIndexError::new_err(message),
        ViewError::NotDrawn(tool) => match tool.code() {
            ErrorCode::Failure(Failure::Timeout) => PyTimeoutError::new_err(tool.message().to_owned()),
            ErrorCode::Refusal(_) => ViewRefused::new_err((
                tool.code().as_str(),
                tool.message().to_owned(),
                tool.to_json().to_string(),
            )),
            ErrorCode::Failure(_) => PyRuntimeError::new_err(message),
        },
    }
}

/// The Word document `doc` of `session` and the engine its pages are laid out with.
fn engine(session: &HostSession, doc: &str) -> PyResult<(DocId, std::sync::Arc<ooxml_tools_word::EnginePages>)> {
    let not_open = || SessionError::NotOpen {
        id: doc.to_owned(),
        open: session.session().documents().iter().map(ToString::to_string).collect(),
    };
    let id = DocId::parse(doc).ok_or_else(|| to_py(&not_open()))?;
    let pages = session.session().pages(&id).map_err(|e| to_py(&e))?;
    let pages =
        pages.ok_or_else(|| PyValueError::new_err(format!("{doc} is not a Word document the host lays out")))?;
    Ok((id, pages))
}

/// `HostSession.view`: see its docs there.
pub(crate) fn view<'py>(
    py: Python<'py>,
    session: &HostSession,
    (doc, since): (&str, Option<&str>),
) -> PyResult<Bound<'py, PyDict>> {
    let (id, pages) = engine(session, doc)?;
    let context = session.application_context();
    let view = py
        .detach(|| session.session().view(&id, since, &*pages, &context))
        .map_err(|e| raised(&e))?;
    view_dict(py, &view)
}

fn view_dict<'py>(py: Python<'py>, view: &View) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    out.set_item("token", &view.token)?;
    let pages: Vec<Bound<'py, PyTuple>> = view
        .pages
        .iter()
        .map(|p| (p.number, PyTuple::new(py, &p.blocks)?, p.width, p.height).into_pyobject(py))
        .collect::<PyResult<_>>()?;
    out.set_item("pages", pages)?;
    out.set_item("changed", &view.changed)?;
    out.set_item("since", &view.since)?;
    out.set_item("stale", view.stale)?;
    out.set_item("note", &view.note)?;
    out.set_item("version", view.version)?;
    Ok(out)
}

/// `HostSession.page_svg`: see its docs there.
pub(crate) fn page_svg(
    py: Python<'_>,
    session: &HostSession,
    (doc, page, token): (&str, usize, &str),
    text: &str,
) -> PyResult<(String, Vec<String>)> {
    let text = TextMode::parse(text).map_err(|e| raised(&e))?;
    let (id, pages) = engine(session, doc)?;
    let context = session.application_context();
    let drawn = py
        .detach(|| session.session().page_svg(&id, page, token, &*pages, text, &context))
        .map_err(|e| raised(&e))?;
    Ok((drawn.svg, drawn.warnings))
}

/// A host's refusal or failure of the changes feed as Python raises it.
fn feed_raised(error: &ooxml_tools::ToolError) -> PyErr {
    match error.code() {
        ErrorCode::Failure(Failure::Timeout) => PyTimeoutError::new_err(error.message().to_owned()),
        ErrorCode::Refusal(_) => ViewRefused::new_err((
            error.code().as_str(),
            error.message().to_owned(),
            error.to_json().to_string(),
        )),
        ErrorCode::Failure(_) => PyRuntimeError::new_err(error.message().to_owned()),
    }
}

/// A picture of the feed's: its address, media type and bytes.
pub(crate) type PictureRow = (String, String, Py<pyo3::types::PyBytes>);

/// `HostSession.changes`: see its docs there.
pub(crate) fn changes(
    py: Python<'_>,
    session: &HostSession,
    (doc, since): (&str, Option<&str>),
    pages: bool,
) -> PyResult<(String, Vec<PictureRow>)> {
    let answer = py
        .detach(|| session.changes(doc, since, pages))
        .map_err(|e| feed_raised(&e))?;
    let pictures = answer
        .pictures
        .iter()
        .map(|p| {
            (
                p.hash.clone(),
                p.mime.clone(),
                pyo3::types::PyBytes::new(py, &p.bytes).unbind(),
            )
        })
        .collect();
    Ok((serde_json::Value::Object(answer.changes).to_string(), pictures))
}
