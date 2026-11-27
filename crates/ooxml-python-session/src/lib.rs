//! The Python module `ooxml_core`'s session API (docs/architecture/README.md, sections 3 and 9;
//! layer 6). **`ooxml_core` is internal** (decision D6, Phase 3): applications and the format
//! libraries use `ooxml_edit.tools` (the Toolbox, its sessions and results), `ooxml_edit.chunks`
//! and the libraries' own APIs; only the facade (ooxml-edit) and docx2svg's renderer import it.
//!
//! Owns: the PyO3 handles -- [`PyHost`] and [`PyHostSession`] (`ooxml_core.Host`,
//! `ooxml_core.HostSession`: the Rust host, `host`, and its live view, `view`), [`PyVersion`]
//! (a document's version), `ooxml_core.tool_specs(kind)` and `ooxml_core.new_document` (`tools`);
//! a saved file's MS-FSSHTTPD chunks and their delta (`chunks`: `chunk_file`, `FileChunks`,
//! `FileChunk`, `ChunkDelta`); the Word layout's handles (`layout`: `WordFonts`, `WordPages`,
//! `LaidPages`, lane L4); the conversion of their arguments and results (bytes copied in while
//! the interpreter lock is held, then every Rust call made with it released, `py.detach`); the
//! mapping of session errors to Python exceptions (`errors`); and the adapters of the ports a
//! session is given (`adapters`: the system clock, no fonts, no part store, an unlimited cache
//! budget).
//!
//! Must not: hold logic beyond conversion (the session and the tools do the work), rely
//! on the interpreter lock for safety (the module is declared free-threading safe: every
//! handle is immutable or synchronised in Rust), call back into Python, or hold
//! process-level state other than what Python's module object holds.
//!
//! The module object is `crates/python`'s (the `ooxml-python` crate, the extension module
//! itself): [`register`] adds these names to it.

mod adapters;
mod budget;
mod chunks;
mod drawn;
mod errors;
mod fonts;
mod host;
mod layout;
mod layout_rows;
mod locale;
mod pages;
mod paths;
mod raster;
mod summary;
mod tools;
mod values;
mod version;
mod view;

use pyo3::prelude::*;

pub use budget::{PyBudgetAccount, PyMemoryBudget};
pub use fonts::PyWordFonts;
pub use host::{PyHost, PyHostSession};
pub use layout::{PyLaidPages, PyWordPages};
pub use pages::PyPageSetup;
pub use raster::PyPageRaster;
pub use version::PyVersion;

/// Add the session API to the module `m`: `Host`, `HostSession`, `OpenRefused`, `Version`,
/// `tool_specs`, `new_document`; the Word layout's handles: `WordFonts`, `WordPages`,
/// `PageSetup`, `LaidPages`, `LayoutSummary`, `MemoryBudget` and `BudgetAccount`; `PageRaster`,
/// `drawing_plan` and `element_path` (docx2svg's drawing and paths); `chunk_file` with
/// `FileChunks`, `FileChunk` and `ChunkDelta`; the live view's `UnknownToken` and
/// `ViewRefused`; and the refusals of an open, `StateRefused` and `NotWordPackage`.
///
/// # Errors
/// Python's, adding a name.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyHost>()?;
    m.add_class::<PyHostSession>()?;
    m.add("OpenRefused", m.py().get_type::<host::OpenRefused>())?;
    m.add_class::<PyVersion>()?;
    m.add_function(wrap_pyfunction!(tools::tool_specs, m)?)?;
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
    m.add_function(wrap_pyfunction!(chunks::chunk_file, m)?)?;
    m.add_class::<chunks::PyFileChunks>()?;
    m.add_class::<chunks::PyFileChunk>()?;
    m.add_class::<chunks::PyChunkDelta>()?;
    m.add("UnknownToken", m.py().get_type::<view::UnknownToken>())?;
    m.add("ViewRefused", m.py().get_type::<view::ViewRefused>())?;
    m.add("StateRefused", m.py().get_type::<errors::StateRefused>())?;
    m.add("NotWordPackage", m.py().get_type::<errors::NotWordPackage>())?;
    Ok(())
}
