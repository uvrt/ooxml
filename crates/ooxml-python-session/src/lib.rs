//! The session API of the Python module `ooxml_core` (docs/architecture/README.md,
//! sections 3 and 9; layer 6).
//!
//! Owns: the PyO3 handles -- [`PySession`] (`ooxml_core.Session`), [`PyCancelToken`]
//! (`ooxml_core.CancelToken`), [`PyLimits`] (`ooxml_core.Limits`) and [`PyCallContext`] (`ooxml_core.CallContext`), [`PyCallOptions`] (`ooxml_core.CallOptions`, what one call runs with) [`PyCallDocuments`] (`ooxml_core.CallDocuments`, the open documents it names), [`PyCallLayout`] (`ooxml_core.CallLayout`, the layout it reads) and [`PyCallWriting`] (`ooxml_core.CallWriting`, how it writes) -- and `ooxml_core.tool_specs(kind)` and `tool_hosting(kind)` (`tools`);
//! the Word layout's handles (`layout`: `WordFonts`, `WordPages`, `LaidPages`, lane L4); the live view's
//! (`view`: `Session.view`, `Session.page_svg`, `UnknownToken`, `ViewRefused`); the conversion of their
//! arguments and results (bytes copied in while the interpreter lock is held, then every
//! Rust call made with it released, `py.detach`); the mapping of session errors to Python
//! exceptions (a panic in a call is the session's own `internal` reply, ooxml-session's
//! `panics`; its message the application part's `panic`); and
//! the adapters of the ports a session is given (`adapters`: the system clock, no fonts,
//! no part store, an unlimited cache budget).
//!
//! Must not: hold logic beyond conversion (the session and the tools do the work), rely
//! on the interpreter lock for safety (the module is declared free-threading safe: every
//! handle is immutable or synchronised in Rust), or hold process-level state other than
//! what Python's module object holds.
//!
//! The module object is `crates/python`'s (the `ooxml-python` crate, the extension module
//! itself): [`register`] adds these names to it.

mod adapters;
mod budget;
mod context;
mod drawn;
mod fonts;
mod group;
mod layout;
mod layout_rows;
mod locale;
mod options;
mod pages;
mod paths;
mod raster;
mod session;
mod summary;
mod tools;
mod values;
mod version;
mod view;

use pyo3::prelude::*;

pub use budget::{PyBudgetAccount, PyMemoryBudget};
pub use context::{PyCallContext, PyCancelToken, PyLimits};
pub use fonts::PyWordFonts;
pub use layout::{PyLaidPages, PyWordPages};
pub use options::{PyCallDocuments, PyCallLayout, PyCallOptions, PyCallWriting};
pub use pages::PyPageSetup;
pub use raster::PyPageRaster;
pub use session::PySession;
pub use version::PyVersion;

/// Add the session API to the module `m`: `Session`, `CancelToken`, `Limits`, `CallContext`, `CallOptions`, `CallDocuments`,
/// `CallWriting`, `CallLayout`, `Version` and
/// `tool_specs`, `tool_hosting`, `new_document`; and the Word layout's handles: `WordFonts`, `WordPages`, `LaidPages`,
/// `LayoutSummary`, `MemoryBudget` and `BudgetAccount`; `PageRaster`, `drawing_plan` and
/// `element_path` (docx2svg's drawing and paths); the live view's `UnknownToken` and
/// `ViewRefused`; and the refusals of an open, `StateRefused` and `NotWordPackage`.
///
/// # Errors
/// Python's, adding a name.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySession>()?;
    m.add_class::<PyCancelToken>()?;
    m.add_class::<PyLimits>()?;
    m.add_class::<PyCallContext>()?;
    m.add_class::<PyCallOptions>()?;
    m.add_class::<PyCallDocuments>()?;
    m.add_class::<PyCallWriting>()?;
    m.add_class::<PyCallLayout>()?;
    m.add_class::<PyVersion>()?;
    m.add_class::<group::PyStepGroup>()?;
    m.add_class::<group::PyGroupMark>()?;
    m.add_function(wrap_pyfunction!(tools::tool_specs, m)?)?;
    m.add_function(wrap_pyfunction!(tools::tool_hosting, m)?)?;
    m.add_function(wrap_pyfunction!(tools::new_document, m)?)?;
    m.add_class::<PyWordFonts>()?;
    m.add_class::<PyWordPages>()?;
    m.add_class::<PyPageSetup>()?;
    m.add_class::<PyLaidPages>()?;
    m.add_class::<PyMemoryBudget>()?;
    m.add_class::<PyBudgetAccount>()?;
    m.add_class::<summary::PyLayoutSummary>()?;
    m.add_class::<PyPageRaster>()?;
    m.add_function(wrap_pyfunction!(raster::drawing_plan, m)?)?;
    m.add_function(wrap_pyfunction!(paths::element_path, m)?)?;
    m.add("UnknownToken", m.py().get_type::<view::UnknownToken>())?;
    m.add("ViewRefused", m.py().get_type::<view::ViewRefused>())?;
    m.add("StateRefused", m.py().get_type::<session::StateRefused>())?;
    m.add("NotWordPackage", m.py().get_type::<session::NotWordPackage>())?;
    Ok(())
}
