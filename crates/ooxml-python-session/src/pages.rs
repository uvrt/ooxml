//! Owns: `ooxml_core.PageSetup` (lane H7b), what a session lays its Word documents out with
//! -- a machine's fonts (`WordFonts`), how the engines lay out and the memory budget they
//! count in -- as `Session(pages=...)` takes it for every document, and `Session.set_pages`
//! for one; and how an engine lays out, as `WordPages` and `PageSetup` take it.
//!
//! Must not: lay out, or hold a Python object (the fonts and the budget are Rust handles).

use crate::budget::{PyMemoryBudget, OWN_BUDGET};
use crate::fonts::PyWordFonts;
use ooxml_base::Locale;
use ooxml_session::MemoryBudget;
use ooxml_session_word::SessionPages;
use ooxml_tools_word::PageSetup;
use ooxml_word_layout::paint::GlyphSize;
use ooxml_word_pages::EngineOptions;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::sync::Arc;

/// What a session lays its Word documents out with: each document gets an engine of its own
/// from it, kept by the session.
#[pyclass(frozen, name = "PageSetup", module = "ooxml_core")]
pub struct PyPageSetup {
    pub(crate) inner: SessionPages,
}

#[pymethods]
impl PyPageSetup {
    /// Engines laying out with `fonts`, under `locale` (a BCP 47 tag), lining decimal tabs
    /// up on `decimal`, drawing glyphs at `glyph_size` (`"device"` or `"exact"`); a chart's
    /// numbers and dates written with `decimal` and `numbers` -- `(thousands, date_order,
    /// list_separator)` -- in `locale`'s language; their memory counted in `budget` (a
    /// `MemoryBudget`; else one of their own) -- as `WordPages` takes them; `raster` (a
    /// `PageRaster`) the faces `render` draws the pages with (`None`: `render` refused as not
    /// supported, unless a call's layout hands its own).
    #[new]
    #[pyo3(signature = (fonts, *, locale = "en-US", decimal = '.', glyph_size = "device",
                        numbers = (',', "mdy".to_owned(), ','), budget = None, raster = None))]
    fn new(
        fonts: PyRef<'_, PyWordFonts>,
        locale: &str,
        decimal: char,
        glyph_size: &str,
        numbers: (char, String, char),
        budget: Option<PyRef<'_, PyMemoryBudget>>,
        raster: Option<PyRef<'_, crate::raster::PyPageRaster>>,
    ) -> PyResult<Self> {
        let options = engine_options(locale, decimal, glyph_size, numbers)?;
        let memory = budget.map_or_else(|| MemoryBudget::new(OWN_BUDGET), |b| Arc::clone(&b.inner));
        Ok(Self {
            inner: SessionPages {
                setup: PageSetup::new(fonts.setup(), options),
                memory,
                raster: raster.map(|r| Arc::clone(&r.inner)),
            },
        })
    }
}

/// How an engine lays out: under `locale` (a BCP 47 tag), lining decimal tabs up on
/// `decimal`, drawing glyphs at `glyph_size` (`"device"` or `"exact"`), a chart's numbers
/// and dates written with `decimal` and `numbers` -- `(thousands, date_order,
/// list_separator)` -- in `locale`'s language.
pub(crate) fn engine_options(
    locale: &str,
    decimal: char,
    glyph_size: &str,
    numbers: (char, String, char),
) -> PyResult<EngineOptions> {
    let (thousands, date_order, list_separator) = numbers;
    let numbers = ooxml_numfmt::Locale::new(decimal, thousands, &date_order, locale, list_separator)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let locale = Locale::from(&numbers);
    let glyph_size = match glyph_size {
        "device" => GlyphSize::Device,
        "exact" => GlyphSize::Exact,
        other => return Err(PyValueError::new_err(format!("glyph_size {other:?}: device or exact"))),
    };
    Ok(EngineOptions {
        locale,
        numbers,
        decimal,
        glyph_size,
        ..EngineOptions::default()
    })
}
