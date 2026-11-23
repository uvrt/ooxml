//! Owns: a laid version's pages drawn for `ooxml_core.LaidPages` (lane svg-thin, docx2svg's
//! thin layer) -- a page's SVG at a size (`svg`), pages as PNG with the faces they draw
//! (`pngs`, as the `render` tool draws them), the files a rasteriser is handed for them
//! (`raster_files`: docx2svg's text as outlines), and the version's coverage as a dict
//! (`coverage_summary`).
//!
//! Must not: lay out, hold the engine's lock while rasterising, or touch a Python object
//! without the interpreter lock (the work is done with it released, then converted).

use ooxml_word_pages::coverage::Coverage;
use ooxml_word_pages::png::faces::{RasterFaces, RasterFile};
use ooxml_word_pages::png::sized::resized;
use ooxml_word_pages::png::PageRaster;
use ooxml_word_pages::{lock_engine, DiskFonts, LaidVersion, PageEngine};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyString};
use std::sync::Mutex;

/// A size asked for: `(width, height)` pixels, either or both.
pub(crate) type Size = (Option<i64>, Option<i64>);

/// A page drawn: its PNG, and what it does not draw as Word draws it.
pub(crate) type DrawnPage = (Vec<u8>, Vec<String>);

/// Why a page could not be drawn.
fn not_painted(version: &LaidVersion, page: usize) -> String {
    format!("page {page} not painted (0-based, of {})", version.page_count())
}

/// Page `page` (0-based) of `version` as SVG at `size` (its natural size in points where no
/// side is given), and what is not drawn as Word draws it.
///
/// # Errors
/// The page is not one of the version's, or its SVG names no size to replace.
pub(crate) fn svg(
    engine: &Mutex<PageEngine>,
    version: &LaidVersion,
    page: usize,
    size: Size,
) -> Result<(String, Vec<String>), String> {
    let mut engine = lock_engine(engine);
    let (svg, missing) = engine
        .page_svg(version, page)
        .ok_or_else(|| not_painted(version, page))?;
    if size == (None, None) {
        return Ok((svg, missing));
    }
    let frame = engine
        .paint(version, page)
        .ok_or_else(|| not_painted(version, page))?
        .geometry
        .frame;
    let (sized, _, _) =
        resized(&svg, &frame, size.0, size.1).ok_or_else(|| format!("page {page}'s SVG names no size"))?;
    Ok((sized, missing))
}

/// `version`'s pages `pages` (0-based) as PNG, each at `size` (300 dpi, its device pixels,
/// where no side is given), drawn by a raster made from `base` with the faces they draw --
/// the system's fonts handed over as `system` says (`None`: only where a face is missing) --
/// and what each does not draw as Word draws it.
///
/// # Errors
/// A page is not one of the version's, or cannot be sized or drawn.
pub(crate) fn pngs(
    engine: &Mutex<PageEngine>,
    version: &LaidVersion,
    (pages, base): (&[usize], &PageRaster),
    size: Size,
    system: Option<bool>,
) -> Result<Vec<DrawnPage>, String> {
    let (svgs, faces) = {
        let mut engine = lock_engine(engine);
        let mut svgs = Vec::with_capacity(pages.len());
        for &page in pages {
            let (svg, missing) = engine
                .page_svg(version, page)
                .ok_or_else(|| not_painted(version, page))?;
            let frame = engine
                .paint(version, page)
                .ok_or_else(|| not_painted(version, page))?
                .geometry
                .frame;
            let size = match size {
                (None, None) => (Some(frame.width_px), Some(frame.height_px)),
                given => given,
            };
            let (sized, _, _) =
                resized(&svg, &frame, size.0, size.1).ok_or_else(|| format!("page {page}'s SVG names no size"))?;
            svgs.push((sized, missing));
        }
        let mut faces = engine.raster_faces(version, pages);
        faces.system = system;
        (svgs, faces)
    };
    let raster = base.for_faces(&DiskFonts, &faces);
    let wanted: Vec<&str> = svgs.iter().map(|(svg, _)| svg.as_str()).collect();
    raster
        .png_all(&wanted)
        .into_iter()
        .zip(svgs)
        .zip(pages)
        .map(|((png, (_, missing)), page)| {
            png.map(|png| (png, missing))
                .map_err(|e| format!("page {page} could not be drawn: {e}"))
        })
        .collect()
}

/// The faces a rasteriser is handed to draw `pages` (0-based) of `version`.
pub(crate) fn raster_faces(engine: &Mutex<PageEngine>, version: &LaidVersion, pages: &[usize]) -> RasterFaces {
    lock_engine(engine).raster_faces(version, pages)
}

/// `faces` for Python: `(files, sans_serif, skip_system, missing)`, each file a path (`str`)
/// or an embedded face's bytes.
pub(crate) fn raster_faces_row<'py>(py: Python<'py>, faces: &RasterFaces) -> PyResult<Bound<'py, PyAny>> {
    let files: Vec<Bound<'py, PyAny>> = faces
        .files
        .iter()
        .map(|file| match file {
            RasterFile::Path(path) => PyString::new(py, path).into_any(),
            RasterFile::Embedded(bytes) => PyBytes::new(py, bytes).into_any(),
        })
        .collect();
    let row = (
        files,
        faces.sans_serif.as_deref(),
        faces.skip_system(),
        faces.missing.clone(),
    );
    Ok(row.into_pyobject(py)?.into_any())
}

/// `coverage` as docx2svg's `Coverage` reads it: `complete`, `pages`, `blocks`, `laid_out`,
/// `substituted` (`(family, substitute, metric_compatible, kind, class)`), `missing`,
/// `approximations` (`(code, reason, message, page, path)`), `placeholders` (the same) and
/// `warnings` (`(code, message, page)`), each in docx2svg's order.
pub(crate) fn coverage_dict<'py>(py: Python<'py>, coverage: &Coverage) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("complete", coverage.complete)?;
    d.set_item("pages", coverage.pages)?;
    d.set_item("blocks", coverage.blocks)?;
    d.set_item("laid_out", coverage.laid_out)?;
    let substituted: Vec<_> = coverage
        .substituted
        .iter()
        .map(|s| {
            (
                s.family.as_str(),
                s.substitute.as_str(),
                s.metric_compatible,
                s.kind,
                s.class,
            )
        })
        .collect();
    d.set_item("substituted", substituted)?;
    d.set_item("missing", coverage.missing.clone())?;
    let approximations: Vec<_> = coverage
        .approximations
        .iter()
        .map(|a| {
            let (reason, message) = a.listed();
            (a.code.as_str(), reason, message, a.page, a.path.as_deref())
        })
        .collect();
    d.set_item("approximations", approximations)?;
    let placeholders: Vec<_> = coverage
        .placeholders
        .iter()
        .map(|p| {
            (
                p.code.as_str(),
                p.reason(),
                p.message.as_str(),
                p.page,
                p.path.as_deref(),
            )
        })
        .collect();
    d.set_item("placeholders", placeholders)?;
    let warnings: Vec<_> = coverage
        .warnings
        .iter()
        .map(|w| (w.code.as_str(), w.message.as_str(), w.page))
        .collect();
    d.set_item("warnings", warnings)?;
    Ok(d)
}
