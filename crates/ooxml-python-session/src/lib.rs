//! The session API of the Python module `ooxml_core` (docs/architecture/README.md,
//! sections 3 and 9; layer 6).
//!
//! Owns: the PyO3 handles -- [`PySession`] (`ooxml_core.Session`), [`PyCancelToken`]
//! (`ooxml_core.CancelToken`) and [`PyCallContext`] (`ooxml_core.CallContext`), [`PyCallOptions`] (`ooxml_core.CallOptions`, what one call runs with) and [`PyCallDocuments`] (`ooxml_core.CallDocuments`, the open documents it names) -- and `ooxml_core.tool_specs(kind)` and `tool_hosting(kind)` (`tools`);
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
mod budget;
mod group;
mod layout;
mod options;
mod raster;
mod session;
mod summary;
mod tools;

use pyo3::prelude::*;

pub use budget::{PyBudgetAccount, PyMemoryBudget};
pub use layout::{LayoutFallback, PyLaidPages, PyWordFonts, PyWordPages};
pub use options::{PyCallDocuments, PyCallOptions};
pub use raster::PyPageRaster;
pub use session::{PyCallContext, PyCancelToken, PySession, PyVersion};

/// Add the session API to the module `m`: `Session`, `CancelToken`, `CallContext`, `CallOptions`, `Version` and
/// `tool_specs`, `tool_hosting`; and the Word layout's handles: `WordFonts`, `WordPages`, `LaidPages`,
/// `LayoutFallback`, `LayoutSummary`, `MemoryBudget` and `BudgetAccount`.
///
/// # Errors
/// Python's, adding a name.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySession>()?;
    m.add_class::<PyCancelToken>()?;
    m.add_class::<PyCallContext>()?;
    m.add_class::<PyCallOptions>()?;
    m.add_class::<PyCallDocuments>()?;
    m.add_class::<PyVersion>()?;
    m.add_class::<group::PyStepGroup>()?;
    m.add_class::<group::PyGroupMark>()?;
    m.add_function(wrap_pyfunction!(tools::tool_specs, m)?)?;
    m.add_function(wrap_pyfunction!(tools::tool_hosting, m)?)?;
    m.add_class::<PyWordFonts>()?;
    m.add_class::<PyWordPages>()?;
    m.add_class::<PyLaidPages>()?;
    m.add_class::<PyMemoryBudget>()?;
    m.add_class::<PyBudgetAccount>()?;
    m.add_class::<summary::PyLayoutSummary>()?;
    m.add_class::<PyPageRaster>()?;
    m.add("LayoutFallback", m.py().get_type::<LayoutFallback>())?;
    Ok(())
}
