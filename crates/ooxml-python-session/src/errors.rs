//! Owns: the mapping of session errors to Python exceptions (`ooxml_core.StateRefused`,
//! `ooxml_core.NotWordPackage`, and the built-in ones), a document kind and an id state read
//! from Python values.
//!
//! Must not: hold a Python object past a call.

use ooxml_session::{DocKind, SessionError};
use ooxml_session_word::IdStateError;
use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyTimeoutError, PyValueError};
use pyo3::prelude::*;

pyo3::create_exception!(
    ooxml_core,
    StateRefused,
    PyValueError,
    "An id state refused as the state of other bytes, of a later form, damaged, or no state: \
its message says what to pass instead (docx-agent's words)."
);

pyo3::create_exception!(
    ooxml_core,
    NotWordPackage,
    PyValueError,
    "A package opened as a Word document without a Word document's main part."
);

/// `kind` (`docx`, `pptx`, `xlsx`), else a `ValueError` naming it.
pub(crate) fn kind_of(kind: &str) -> PyResult<DocKind> {
    DocKind::parse(kind).ok_or_else(|| to_py(&SessionError::Kind(kind.to_owned())))
}

/// `error` as Python raises it.
pub(crate) fn to_py(error: &SessionError) -> PyErr {
    let message = error.to_string();
    if ooxml_session_word::refuses_state(error) {
        return StateRefused::new_err(message);
    }
    if ooxml_session_word::not_word(error) {
        return NotWordPackage::new_err(message);
    }
    match error {
        SessionError::StateRefused { .. } => StateRefused::new_err(message),
        SessionError::IdTaken(_) => PyValueError::new_err(message),
        SessionError::NotOpen { .. } => PyKeyError::new_err(message),
        SessionError::Kind(_) | SessionError::PartName(_) | SessionError::Package(_) => PyValueError::new_err(message),
        SessionError::Base(ooxml_base::BaseError::Cancelled) => PyTimeoutError::new_err(message),
        SessionError::Format(_) => PyValueError::new_err(message),
        SessionError::Entangled { .. } | SessionError::RedoNotNext { .. } | SessionError::NewProblems { .. } => {
            PyValueError::new_err(message)
        }
        SessionError::Store(_) | SessionError::Base(_) => PyRuntimeError::new_err(message),
    }
}

/// A state's text as docx-agent's `load_state` reads it: a string, or bytes read as ASCII;
/// any other value (bytes that are not ASCII too) refused, named by its type.
pub(crate) fn state_text(state: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(text) = state.extract::<String>() {
        return Ok(text);
    }
    let type_name = match state.extract::<Vec<u8>>() {
        Ok(bytes) if bytes.is_ascii() => return Ok(String::from_utf8(bytes).unwrap_or_default()),
        Ok(_) => "NoneType".to_owned(),
        Err(_) => state.get_type().name()?.to_string(),
    };
    Err(StateRefused::new_err(IdStateError::NotAState(type_name).to_string()))
}
