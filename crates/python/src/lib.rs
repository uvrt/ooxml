//! The Python module `ooxml_core`: the Rust session's API (ooxml-python-session), one native
//! module for every package that drives it.
use pyo3::prelude::*;

// Free-threading safe: ooxml-python-session's classes are frozen, shared state behind Arc and locks.
#[pymodule(gil_used = false)]
fn ooxml_core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    ooxml_python_session::register(m)
}
