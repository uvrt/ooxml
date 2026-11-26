//! Owns: the Word layout's Python handles (lane L4) -- `ooxml_core.WordPages` (one
//! document's page engine: its versions laid out, each from the one before) and
//! `ooxml_core.LaidPages` (one version laid out: its pages, facts, page numbers, page views,
//! SVG and PNG, and coverage; a version the layout does not lay out yet raises
//! `NotImplementedError`, naming why and where). Exact lengths cross as `(numerator,
//! denominator)` pairs (`layout_rows`); the fonts are `fonts`' `WordFonts`.
//!
//! Must not: lay out or draw (ooxml-word-pages does), hold a Python object beyond a call
//! (a painter is called while its call lasts), or touch one without the interpreter lock:
//! every Rust call runs with it released.

use crate::budget::{PyMemoryBudget, OWN_BUDGET};
use crate::fonts::PyWordFonts;
use crate::layout_rows::{exact, grade_row, page_row, view_dict, CoverageRows, Exact, PageRow};
use crate::pages::engine_options;
use crate::raster::PyPageRaster;
use ooxml_session::MemoryBudget;
use ooxml_word_layout::facts::BlockKeeps;
use ooxml_word_layout::fields::update::PageNumbers;
use ooxml_word_pages::{EngineBudgets, LaidVersion, PageEngine};
use pyo3::exceptions::{PyNotImplementedError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use std::sync::{Arc, Mutex};

/// A line's digest row (see `LaidPages.digests`).
type LineDigest<'py> = (String, Option<&'static str>, Option<String>, Bound<'py, PyBytes>);

/// A line's text as a check reads it (see `LaidPages.line_texts`).
type LineTextRow = (
    String,
    Option<&'static str>,
    Option<String>,
    usize,
    usize,
    Vec<(i64, usize)>,
    Option<(Exact, Exact)>,
    Option<Exact>,
);

/// A body line as a check reads it: `(path, number, top, pitch, baseline, column)`.
type BodyLine = (String, usize, Exact, Exact, i64, Option<usize>, Option<String>);

/// One document's page engine: its versions laid out, each from the one before.
#[pyclass(frozen, name = "WordPages", module = "ooxml_core")]
pub struct PyWordPages {
    engine: Arc<Mutex<PageEngine>>,
    /// The engine's seed, taken when it was made: read without waiting for the engine.
    seed: ooxml_word_pages::EngineSeed,
}

fn lock(engine: &Mutex<PageEngine>) -> std::sync::MutexGuard<'_, PageEngine> {
    ooxml_word_pages::lock_engine(engine)
}

type Laid = Result<Result<Arc<LaidVersion>, ooxml_word_pages::Fallback>, ooxml_word_pages::PagesError>;

fn laid_or_raise(engine: &Arc<Mutex<PageEngine>>, laid: Laid) -> PyResult<PyLaidPages> {
    match laid {
        Ok(Ok(version)) => Ok(PyLaidPages {
            version,
            engine: Arc::clone(engine),
        }),
        Ok(Err(unsupported)) => Err(PyNotImplementedError::new_err(format!(
            "the Word layout does not lay this version out yet: {unsupported}"
        ))),
        Err(error) => Err(PyValueError::new_err(error.to_string())),
    }
}

impl PyWordPages {
    /// The handle of an engine the session keeps (`Session.pages`): the same engine.
    pub(crate) fn of(pages: &ooxml_tools_word::EnginePages) -> Self {
        use ooxml_tools_word::WordLayout;
        Self {
            engine: Arc::clone(pages.engine()),
            seed: pages.seed().clone(),
        }
    }

    /// The edit core's layout port over this engine (a call's `layout`).
    pub(crate) fn port(&self) -> ooxml_tools_word::EnginePages {
        ooxml_tools_word::EnginePages::new(Arc::clone(&self.engine), self.seed.clone())
    }
}

#[pymethods]
impl PyWordPages {
    /// An engine laying out with `fonts`, under `locale` (a BCP 47 tag), lining decimal
    /// tabs up on `decimal`, drawing glyphs at `glyph_size` (`"device"` or `"exact"`); a
    /// chart's numbers and dates written with `decimal` and `numbers` -- `(thousands,
    /// date_order, list_separator)` -- in `locale`'s language (ooxml-common's `Locale`); its
    /// memory counted in `budget` (a `MemoryBudget`; else one of its own, for its pages'
    /// paint alone).
    #[new]
    #[pyo3(signature = (fonts, *, locale = "en-US", decimal = '.', glyph_size = "device",
                        numbers = (',', "mdy".to_owned(), ','), budget = None))]
    fn new(
        fonts: PyRef<'_, PyWordFonts>,
        locale: &str,
        decimal: char,
        glyph_size: &str,
        numbers: (char, String, char),
        budget: Option<PyRef<'_, PyMemoryBudget>>,
    ) -> PyResult<Self> {
        let options = engine_options(locale, decimal, glyph_size, numbers)?;
        let budget = budget.map_or_else(|| MemoryBudget::new(OWN_BUDGET), |b| Arc::clone(&b.inner));
        let budgets = EngineBudgets {
            held: budget.holding(),
            paint: budget.account(),
        };
        let engine = PageEngine::new(fonts.setup(), options, budgets);
        Ok(Self {
            seed: engine.seed(),
            engine: Arc::new(Mutex::new(engine)),
        })
    }

    /// Lay out the version whose package is `data`, from the last version laid out: only
    /// the parts whose stored bytes changed are read again. Raises `NotImplementedError` for a
    /// version the layout does not lay out yet (why and where).
    fn lay_out(&self, py: Python<'_>, data: &[u8]) -> PyResult<PyLaidPages> {
        let bytes: Arc<[u8]> = Arc::from(data);
        let engine = &self.engine;
        let laid = py.detach(|| lock(engine).lay_out_bytes(bytes));
        laid_or_raise(engine, laid)
    }

    /// Lay out the last version with `parts` (name -> bytes, or `None` to remove the part)
    /// put in place: a version's touched parts.
    fn update(&self, py: Python<'_>, parts: &Bound<'_, PyDict>) -> PyResult<PyLaidPages> {
        let mut copied = Vec::with_capacity(parts.len());
        for (name, data) in parts.iter() {
            let name: String = name.extract()?;
            let data: Option<Vec<u8>> = data.extract()?;
            copied.push((name, data.map(Arc::<[u8]>::from)));
        }
        let engine = &self.engine;
        let laid = py.detach(|| lock(engine).update(&copied));
        laid_or_raise(engine, laid)
    }

    /// Let go of the last version and the memos: the next version is laid out whole.
    fn forget(&self, py: Python<'_>) {
        let engine = &self.engine;
        py.detach(|| lock(engine).forget());
    }

    /// What the engine did: `(name, value)` pairs (counts, times in seconds, and the
    /// bytes its memos and its pages' paint hold now).
    fn stats(&self, py: Python<'_>) -> Vec<(&'static str, f64)> {
        let engine = &self.engine;
        let (s, (held, painted), digests) = py.detach(|| {
            let engine = lock(engine);
            (engine.stats(), engine.memory(), engine.digest_cache())
        });
        #[allow(clippy::cast_precision_loss)]
        let n = |v: u64| v as f64;
        vec![
            ("full", n(s.full)),
            ("relaid", n(s.relaid)),
            ("kept", n(s.kept)),
            ("fallbacks", n(s.fallbacks)),
            ("parts_read", n(s.parts_read)),
            ("parts_kept", n(s.parts_kept)),
            ("pages_painted", n(s.pages_painted)),
            ("swept", n(s.swept)),
            ("paint_hits", n(s.paint_hits)),
            ("pages_said_again", n(s.pages_said.0)),
            ("pages_walked", n(s.pages_said.1)),
            ("memo_bytes", held as f64),
            ("paint_bytes", painted as f64),
            ("pages_digested", n(s.pages_digested.0)),
            ("pages_taken_digested", n(s.pages_digested.1)),
            ("digest_pages", digests.0 as f64),
            ("digest_bytes", digests.1 as f64),
            ("open_seconds", s.open_time.as_secs_f64()),
            ("layout_seconds", s.layout_time.as_secs_f64()),
        ]
    }
}

/// One version laid out.
#[pyclass(frozen, name = "LaidPages", module = "ooxml_core")]
pub struct PyLaidPages {
    version: Arc<LaidVersion>,
    engine: Arc<Mutex<PageEngine>>,
}

#[pymethods]
impl PyLaidPages {
    /// Its pages, blank ones included.
    #[getter]
    fn page_count(&self) -> usize {
        self.version.page_count()
    }

    /// Whether it was laid out from the version before.
    #[getter]
    fn relaid(&self) -> bool {
        self.version.relaid()
    }

    /// The body's top-level blocks (paragraphs and tables), the endnotes placed in the flow
    /// left out: what docx2svg's coverage counts.
    #[getter]
    fn body_blocks(&self) -> usize {
        let layout = self.version.layout();
        layout.roles.iter().filter(|role| role.is_none()).count()
    }

    /// The body blocks laid out: all but those drawn as placeholders (docx2svg's
    /// `Coverage.blocks_laid_out`).
    #[getter]
    fn blocks_laid_out(&self) -> usize {
        ooxml_word_pages::coverage::blocks_laid_out(&self.version).0
    }

    /// What it holds in bytes, roughly.
    #[getter]
    fn weight(&self) -> usize {
        self.version.weight()
    }

    /// Every page's frame and what the paginator knows of it: `((number, section,
    /// section_first, story, blank), (width_px, height_px, width_pt, height_pt, (left, right,
    /// top, bottom)))` (docx2svg's `Page.column`), the points and the box exact.
    fn pages(&self, py: Python<'_>) -> Vec<PageRow> {
        let version = &self.version;
        py.detach(|| {
            version
                .facts()
                .pages
                .iter()
                .map(|p| page_row(&p.info, &p.frame))
                .collect()
        })
    }

    /// The body's lines page by page, as a check reads them: per page a list of `(path,
    /// number, top, pitch, baseline, column, id)`, `id` the paragraph's as the tool names it
    /// in this version's bytes (`None` where the line is no body paragraph's).
    fn body_lines(&self, py: Python<'_>) -> Vec<Vec<BodyLine>> {
        let version = &self.version;
        py.detach(|| {
            version
                .facts()
                .pages
                .iter()
                .map(|p| {
                    p.lines
                        .iter()
                        .map(|l| {
                            (
                                l.path.to_string(),
                                l.number,
                                exact(l.top),
                                exact(l.pitch),
                                l.baseline,
                                l.column,
                                version
                                    .id_named(l.anchor.map(|a| a.block), &l.path)
                                    .map(|id| id.to_string()),
                            )
                        })
                        .collect()
                })
                .collect()
        })
    }

    /// The ids, as the tool names them in this version's bytes, of the body paragraphs,
    /// tables and rows at `paths` (docx2svg's paths): `None` for a path that names none.
    fn ids_of(&self, py: Python<'_>, paths: Vec<String>) -> Vec<Option<String>> {
        let version = &self.version;
        py.detach(|| paths.iter().map(|p| version.id_of(p)).collect())
    }

    /// The body blocks each page draws something of (docx2svg's `Page.blocks`).
    fn blocks(&self, py: Python<'_>) -> Vec<Vec<String>> {
        let version = &self.version;
        py.detach(|| version.facts().pages.iter().map(|p| version.page_blocks(p)).collect())
    }

    /// Each top-level body block's keeps: `{index: ("paragraph", keep_next, keep_lines,
    /// page_break_before) | ("table", rows' keeps, header_rows)}`.
    fn keeps<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let version = &self.version;
        let keeps = py.detach(|| version.facts().keeps.clone());
        let out = PyDict::new(py);
        for (index, keep) in keeps {
            let value = match keep {
                BlockKeeps::Paragraph {
                    keep_next,
                    keep_lines,
                    page_break_before,
                } => ("paragraph", keep_next, keep_lines, page_break_before)
                    .into_pyobject(py)?
                    .into_any(),
                BlockKeeps::Table { rows, header_rows } => ("table", rows, header_rows).into_pyobject(py)?.into_any(),
            };
            out.set_item(index, value)?;
        }
        Ok(out)
    }

    /// The faces it was measured with that were not the ones asked for, as docx2svg's
    /// coverage reads them: `(grades, glyph_fallbacks, warnings)` -- each family's first
    /// lookup, in the order they were first looked up, `(family, grade, substitute, metric_compatible, kind, class)` (`grade` one of
    /// `installed`, `embedded`, `substituted`, `recorded`, `missing`), each `(family,
    /// character, face used)` whose advance came from another face, and the model's
    /// reading warnings (`styles-part-missing`, ...).
    fn coverage(&self, py: Python<'_>) -> CoverageRows {
        let version = &self.version;
        py.detach(|| {
            let coverage = version.glyphs().coverage();
            let grades = coverage
                .order
                .iter()
                .filter_map(|family| coverage.grades.get(family).map(|grade| grade_row(family, grade)))
                .collect();
            let fallbacks = coverage
                .fallback_order
                .iter()
                .filter_map(|key| {
                    let used = coverage.glyph_fallbacks.get(key)?;
                    Some((key.0.clone(), key.1, used.clone()))
                })
                .collect();
            let warnings = version
                .layout()
                .document
                .warnings
                .iter()
                .map(ToString::to_string)
                .collect();
            (grades, fallbacks, warnings)
        })
    }

    /// docx2svg's layout warnings of this version, in its order: `(code, message, 1-based
    /// page or None)`.
    fn warnings(&self) -> Vec<(String, String, Option<usize>)> {
        self.version
            .said()
            .warnings
            .iter()
            .map(|w| (w.code.clone(), w.message.clone(), w.page))
            .collect()
    }

    /// docx2svg's placeholders of this version, drawn where it carried on past what it
    /// cannot lay out: `(code, message, 1-based page, path)`.
    fn placeholders(&self) -> Vec<(String, String, Option<usize>, Option<String>)> {
        self.version
            .said()
            .placeholders
            .iter()
            .map(|p| (p.code.clone(), p.message.clone(), p.page, p.path.clone()))
            .collect()
    }

    /// docx2svg's approximations of this version: `(code, message, 1-based page, path)`.
    fn approximations(&self) -> Vec<(String, String, Option<usize>, Option<String>)> {
        self.version
            .said()
            .approximations
            .iter()
            .map(|a| (a.code.clone(), a.message.clone(), a.page, a.path.clone()))
            .collect()
    }

    /// Why it was laid out whole although a version before was laid out: a dict of
    /// `reason` and, where known, `at`, `para_id` and `part` (docx2svg's `relaid_cause`), or
    /// `None`.
    #[getter]
    fn relaid_cause<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        let Some(cause) = self.version.relaid_cause() else {
            return Ok(None);
        };
        let d = PyDict::new(py);
        d.set_item("reason", cause.reason)?;
        for (key, value) in [("at", &cause.at), ("para_id", &cause.para_id), ("part", &cause.part)] {
            if let Some(value) = value {
                d.set_item(key, value)?;
            }
        }
        Ok(Some(d))
    }

    /// The 0-based block the layout stopped at, if it did.
    #[getter]
    fn stopped(&self, py: Python<'_>) -> Option<usize> {
        let version = &self.version;
        py.detach(|| version.facts().stopped)
    }

    /// What a page-number field (`PAGE`, `NUMPAGES`, `SECTIONPAGES`) in the paragraph at
    /// `path` shows, or a `PAGEREF` (`keyword` `"PAGEREF"`) to the bookmark it names: the
    /// text, or `None` where it is not known.
    fn page_field(&self, py: Python<'_>, instruction: &str, keyword: &str, path: &str) -> Option<String> {
        let version = &self.version;
        py.detach(|| {
            let numbers = PageNumbers::new(version.layout());
            if keyword.eq_ignore_ascii_case("PAGEREF") {
                numbers.page_reference(instruction, path).ok()
            } else {
                numbers.page_field(instruction, keyword, path).ok()
            }
        })
    }

    /// Page `page` (0-based) as docx2svg's `Page` holds it: a dict of its frame (`number`,
    /// `width_px`, `height_px`, `width_pt`, `height_pt`, `column`), `lines` (each `(path,
    /// number, baseline, top, pitch, paragraph_id, band, story, part, column, spans)`, a
    /// span `(path, chars, xs, y, end, face, bold, italic, half_points, color, underline,
    /// strike, double_strike, highlight, shading, kind, line_baseline, vertical_align)`),
    /// `rules` (`(kind, path, x, y, width, height, color, style)`), `pictures` (`(path,
    /// relationship, x, y, width, height, part, crop)`), `drawings` (slots, as a painter
    /// is given them, each with its text boxes' `lines`), `blocks` and `unpainted`. `lines` false: its lines left out (lane
    /// L4b: what a reader of its rules, pictures and drawings alone reads).
    #[pyo3(signature = (page, lines = true))]
    fn page<'py>(&self, py: Python<'py>, page: usize, lines: bool) -> PyResult<Bound<'py, PyDict>> {
        let (version, engine) = (&self.version, &self.engine);
        let view = py
            .detach(|| lock(engine).page(version, page))
            .ok_or_else(|| PyValueError::new_err(format!("no page {page} (0-based) of {}", version.page_count())))?;
        view_dict(py, &view, lines)
    }

    /// What every page paints, digested (bytes), painted here: per page `(rest, lines)`,
    /// `rest` its frame, rules, pictures and drawings, each line `(path, story, part, digest)` --
    /// what tells two versions' pages apart, no page read into Python.
    fn digests<'py>(&self, py: Python<'py>) -> PyResult<Vec<(Bound<'py, PyBytes>, Vec<LineDigest<'py>>)>> {
        let (version, engine) = (&self.version, &self.engine);
        let pages = py.detach(|| {
            let mut engine = lock(engine);
            (0..version.page_count())
                .map(|k| engine.page_digests(version, k))
                .collect::<Option<Vec<_>>>()
        });
        let pages = pages.ok_or_else(|| PyRuntimeError::new_err("a page of another engine generation"))?;
        Ok(pages
            .into_iter()
            .map(|p| {
                let lines = p
                    .lines
                    .into_iter()
                    .map(|(path, story, d)| {
                        let (kind, part) = story.map_or((None, None), |(k, p)| (Some(k), Some(p.to_string())));
                        (path, kind, part, PyBytes::new(py, &d.0))
                    })
                    .collect();
                (PyBytes::new(py, &p.rest.0), lines)
            })
            .collect())
    }

    /// Page `page`'s lines' text as a check reads it (lane L4b), every story's: per line
    /// `(path, story, part, number, inked, sizes, ink, widest)` -- `inked` its characters
    /// that are not white space, `sizes` `[(half_points, chars)]` of its text not raised or
    /// lowered, `ink` `((left), (right))` and `widest` exact device px (`None` for a line of
    /// no ink) -- painted in Rust, none of it read into Python as spans.
    fn line_texts(&self, py: Python<'_>, page: usize) -> PyResult<Vec<LineTextRow>> {
        let (version, engine) = (&self.version, &self.engine);
        let found = py.detach(|| lock(engine).line_texts(version, page));
        let lines = found.ok_or_else(|| PyRuntimeError::new_err("no such page of this engine's generation"))?;
        let names = &version.layout().document.names;
        Ok(lines
            .into_iter()
            .map(|l| {
                let path = l.place.named(names).unwrap_or_default();
                let (story, part) = l.story.map_or((None, None), |(k, p)| (Some(k), Some(p)));
                (
                    path,
                    story,
                    part,
                    l.number,
                    l.inked,
                    l.sizes,
                    l.ink.map(|(a, b)| (exact(a), exact(b))),
                    l.widest.map(exact),
                )
            })
            .collect())
    }

    /// Its layout summary (`LayoutSummary`), made by its engine: the pages the version
    /// before's summary kept taken as they were, the rest painted and digested.
    fn summary(&self, py: Python<'_>) -> PyResult<crate::summary::PyLayoutSummary> {
        let (version, engine) = (&self.version, &self.engine);
        let made = py.detach(|| lock(engine).summary(version));
        let inner = made.ok_or_else(|| PyRuntimeError::new_err("a version of another engine generation"))?;
        Ok(crate::summary::PyLayoutSummary {
            inner: ooxml_word_pages::summarised::Summarised::Made(inner),
        })
    }

    /// Page `page` (0-based) as SVG, byte for byte docx2svg's where the list beside it is
    /// empty, its drawings painted in Rust; else it names what is not drawn as docx2svg
    /// draws it. At `width` and `height` pixels where given (one alone: the other in the
    /// page's shape), else at its natural size in points.
    #[pyo3(signature = (page, width = None, height = None))]
    fn svg(
        &self,
        py: Python<'_>,
        page: usize,
        width: Option<i64>,
        height: Option<i64>,
    ) -> PyResult<(String, Vec<String>)> {
        let (version, engine) = (&self.version, &self.engine);
        py.detach(|| crate::drawn::svg(engine, version, page, (width, height)))
            .map_err(PyRuntimeError::new_err)
    }

    /// Pages `pages` (0-based) as PNG, as the `render` tool draws them: each at `width` and
    /// `height` pixels where given (one alone: the other in the page's shape), else at 300
    /// dpi; by a raster made from `raster` (its files and folders after the faces the pages
    /// draw), the system's fonts handed over as `system` says (`None`: only where a face is
    /// missing). Per page `(png, what is not drawn as Word draws it)`.
    #[pyo3(signature = (pages, raster, *, width = None, height = None, system = None))]
    fn pngs<'py>(
        &self,
        py: Python<'py>,
        pages: Vec<usize>,
        raster: PyRef<'_, PyPageRaster>,
        width: Option<i64>,
        height: Option<i64>,
        system: Option<bool>,
    ) -> PyResult<Vec<(Bound<'py, PyBytes>, Vec<String>)>> {
        let (version, engine, base) = (&self.version, &self.engine, &raster.inner);
        let drawn = py
            .detach(|| crate::drawn::pngs(engine, version, (&pages, base), (width, height), system))
            .map_err(PyRuntimeError::new_err)?;
        Ok(drawn
            .into_iter()
            .map(|(png, missing)| (PyBytes::new(py, &png), missing))
            .collect())
    }

    /// The files a rasteriser is handed to draw `pages` (0-based) as Word draws them:
    /// `(files, sans_serif, skip_system, missing)`, each file a path or the bytes of a face
    /// the document embeds; `sans_serif` the face the generic `sans-serif` names;
    /// `skip_system` whether the system's fonts are left out (no face is `missing`).
    fn raster_files<'py>(&self, py: Python<'py>, pages: Vec<usize>) -> PyResult<Bound<'py, PyAny>> {
        let (version, engine) = (&self.version, &self.engine);
        let faces = py.detach(|| crate::drawn::raster_faces(engine, version, &pages));
        crate::drawn::raster_faces_row(py, &faces)
    }

    /// Its coverage, as docx2svg's `Coverage` reads it (a dict: `complete`, `pages`,
    /// `blocks`, `laid_out`, `substituted`, `missing`, `approximations`, `placeholders`,
    /// `warnings`; see `drawn::coverage_dict`).
    fn coverage_summary<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let (version, engine) = (&self.version, &self.engine);
        let coverage = py
            .detach(|| lock(engine).coverage(version))
            .ok_or_else(|| PyRuntimeError::new_err("a version of another engine generation"))?;
        crate::drawn::coverage_dict(py, &coverage)
    }
}
