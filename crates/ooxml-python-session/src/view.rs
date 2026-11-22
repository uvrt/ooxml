//! Owns: the live view's Python side (`Session.view`, `Session.page_svg`; an application's
//! API, not a tool): a view as a dict, a page as `(svg, warnings)`, every Rust call made with
//! the interpreter lock released, and the mapping of the view's errors --
//! `ooxml_core.UnknownToken` (a `LookupError`), `IndexError` for a page the version does not
//! have, `ValueError` for a text mode there is none of, `ooxml_core.ViewRefused` (args: the
//! refusal's code, its message, its error as JSON text) for a version or a mode not drawn yet,
//! `TimeoutError` past the call's limit on laying out.
//!
//! Must not: do the view's work (ooxml-session does), or hold a Python object while the
//! version is laid out or painted.

use crate::context::{caller_context, PyCallContext};
use crate::options::PyCallLayout;
use crate::session::{doc_id, to_py};
use ooxml_session::{Session, TextMode, View, ViewError};
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

/// `Session.view`: see its docs there.
pub(crate) fn view<'py>(
    py: Python<'py>,
    session: &Session,
    (doc, since): (&str, Option<&str>),
    layout: &PyCallLayout,
    context: Option<&PyCallContext>,
) -> PyResult<Bound<'py, PyDict>> {
    let id = doc_id(doc, session)?;
    let pages = layout.word_layout()?;
    let context = caller_context(context);
    let view = py
        .detach(|| session.view(&id, since, &*pages, &context))
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

/// `Session.page_svg`: see its docs there.
pub(crate) fn page_svg(
    py: Python<'_>,
    session: &Session,
    (doc, page, token): (&str, usize, &str),
    (layout, text): (&PyCallLayout, &str),
    context: Option<&PyCallContext>,
) -> PyResult<(String, Vec<String>)> {
    let text = TextMode::parse(text).map_err(|e| raised(&e))?;
    let id = doc_id(doc, session)?;
    let pages = layout.word_layout()?;
    let context = caller_context(context);
    let drawn = py
        .detach(|| session.page_svg(&id, page, token, &*pages, text, &context))
        .map_err(|e| raised(&e))?;
    Ok((drawn.svg, drawn.warnings))
}
