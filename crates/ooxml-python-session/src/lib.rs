//! The session API of the Python module `ooxml_core` (docs/architecture/README.md,
//! sections 3 and 9; layer 6).
//!
//! Owns: the PyO3 handles -- [`PySession`] (`ooxml_core.Session`), [`PyCancelToken`]
//! (`ooxml_core.CancelToken`) and [`PyCallContext`] (`ooxml_core.CallContext`) -- and `ooxml_core.tool_specs(kind)`;
//! the Word layout's handles (`layout`: `WordFonts`, `WordPages`, `LaidPages`, lane L4); the conversion of their
//! arguments and results (bytes copied in while the interpreter lock is held, then every
//! Rust call made with it released, `py.detach`); the mapping of session errors to Python
//! exceptions; a panic in a call turned into an `internal` tool error, never an abort; and
//! the adapters of the ports a session is given (`adapters`: the system clock, no fonts,
//! no part store, an unlimited cache budget).
//!
//! Must not: hold logic beyond conversion (the session and the tools do the work), rely
//! on the interpreter lock for safety (the module is declared free-threading safe: every
//! handle is immutable or synchronised in Rust), or hold process-level state other than
//! what Python's module object holds.
//!
//! The module object is `crates/python`'s, so one native module ships: [`register`] adds
//! these names to it beside the prototype's. The design calls this crate `ooxml-python`;
//! the prototype's package holds that name until it is retired (section 13), when this
//! crate takes it and becomes the module itself.

mod adapters;
mod layout;
mod session;

use pyo3::prelude::*;

pub use layout::{LayoutFallback, PyBudgetAccount, PyLaidPages, PyMemoryBudget, PyWordFonts, PyWordPages};
pub use session::{PyCallContext, PyCancelToken, PySession};

/// Add the session API to the module `m`: `Session`, `CancelToken`, `CallContext` and
/// `tool_specs`; and the Word layout's handles: `WordFonts`, `WordPages`, `LaidPages`,
/// `LayoutFallback`, `MemoryBudget` and `BudgetAccount`.
///
/// # Errors
/// Python's, adding a name.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySession>()?;
    m.add_class::<PyCancelToken>()?;
    m.add_class::<PyCallContext>()?;
    m.add_function(wrap_pyfunction!(session::tool_specs, m)?)?;
    m.add_class::<PyWordFonts>()?;
    m.add_class::<PyWordPages>()?;
    m.add_class::<PyLaidPages>()?;
    m.add_class::<PyMemoryBudget>()?;
    m.add_class::<PyBudgetAccount>()?;
    m.add("LayoutFallback", m.py().get_type::<LayoutFallback>())?;
    Ok(())
}
