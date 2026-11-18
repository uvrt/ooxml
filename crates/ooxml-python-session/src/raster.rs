//! Owns: a page's PNG for Python (`ooxml_core.PageRaster`): the faces handed over read once
//! (`ooxml_word_pages::png::PageRaster`), each page's SVG drawn with the interpreter lock
//! let go -- what docx2svg's `svg_to_png` draws with resvg-py, in process.
//!
//! Must not: lay out, read the machine's font folders beyond what it is handed (the system's
//! fonts when asked), or hold a Python object.

use ooxml_raster::{Fit, FontLoader, FontSet, Rgba};
use ooxml_word_pages::png::PageRaster;
use ooxml_word_pages::DiskFonts;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

/// The faces a document's pages are drawn with, read once: `font_files`, then the files
/// under `font_dirs`, after the system's fonts unless `skip_system_fonts`;
/// `sans_serif_family` the face the generic `sans-serif` names.
#[pyclass(frozen, name = "PageRaster", module = "ooxml_core")]
pub struct PyPageRaster {
    inner: PageRaster,
}

/// The system's fonts, read once per raster made with them.
fn system_fonts() -> FontSet {
    let mut loader = FontLoader::new(ooxml_raster::Families::platform_default());
    loader.system();
    loader.build()
}

/// `#RRGGBB` (or `#RGB`, `white`, `black`, `transparent`) as a colour.
fn colour(text: &str) -> Option<Rgba> {
    let opaque = |red, green, blue| Rgba {
        red,
        green,
        blue,
        alpha: 255,
    };
    match text.trim().to_ascii_lowercase().as_str() {
        "white" => return Some(opaque(255, 255, 255)),
        "black" => return Some(opaque(0, 0, 0)),
        "transparent" => {
            return Some(Rgba {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 0,
            })
        }
        _ => {}
    }
    let hex = text.trim().strip_prefix('#')?;
    let hex: String = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect(),
        6 => hex.to_owned(),
        _ => return None,
    };
    let channel = |k: usize| u8::from_str_radix(hex.get(k..k + 2)?, 16).ok();
    Some(opaque(channel(0)?, channel(2)?, channel(4)?))
}

#[pymethods]
impl PyPageRaster {
    #[new]
    #[pyo3(signature = (font_files = Vec::new(), font_dirs = Vec::new(), *, skip_system_fonts = false,
                        sans_serif_family = None))]
    fn new(
        py: Python<'_>,
        font_files: Vec<String>,
        font_dirs: Vec<String>,
        skip_system_fonts: bool,
        sans_serif_family: Option<String>,
    ) -> Self {
        let inner = py.detach(|| {
            let system = (!skip_system_fonts).then(system_fonts);
            PageRaster::new(
                &DiskFonts,
                system.as_ref(),
                (&font_files, &font_dirs),
                sans_serif_family.as_deref(),
            )
        });
        Self { inner }
    }

    /// `svg` as PNG bytes: at `width` or `height` pixels (the other in proportion), or
    /// `scale` times its own size, over `background` (a CSS colour) where given.
    #[pyo3(signature = (svg, *, width = None, height = None, scale = None, background = None))]
    fn png<'py>(
        &self,
        py: Python<'py>,
        svg: &str,
        width: Option<u32>,
        height: Option<u32>,
        scale: Option<f32>,
        background: Option<&str>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let fit = match (width, height, scale) {
            (Some(w), Some(h), _) => Fit::Size(w, h),
            (Some(w), None, _) => Fit::Width(w),
            (None, Some(h), _) => Fit::Height(h),
            (None, None, Some(z)) => Fit::Zoom(z),
            (None, None, None) => Fit::Original,
        };
        let background = match background {
            Some(text) => Some(colour(text).ok_or_else(|| PyValueError::new_err(format!("background {text:?}")))?),
            None => None,
        };
        let svg = svg.to_owned();
        let drawn = py.detach(|| self.inner.png(&svg, fit, background));
        let bytes = drawn.map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(PyBytes::new(py, &bytes))
    }
}
