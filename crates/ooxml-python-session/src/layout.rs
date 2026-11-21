//! Owns: the Word layout's Python handles (lane L4) -- `ooxml_core.WordFonts` (a machine's
//! fonts for one set of folders, scanned once), `ooxml_core.WordPages` (one document's page
//! engine: its versions laid out, each from the one before) and `ooxml_core.LaidPages`
//! (one version laid out: its pages, facts, page numbers, page views and SVG), and
//! `ooxml_core.LayoutFallback`, raised for a version the Rust layout does not lay out as
//! docx2svg does yet; and `ooxml_core.MemoryBudget` (lane L4b), the session's memory
//! budget its engines count in and its host's kept layouts reserve from. Exact lengths
//! cross as `(numerator, denominator)` pairs.
//!
//! Must not: lay out or draw (ooxml-word-pages does), hold a Python object beyond a call
//! (a painter is called while its call lasts), or touch one without the interpreter lock:
//! every Rust call runs with it released.

use crate::budget::{PyMemoryBudget, OWN_BUDGET};
use ooxml_base::Locale;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_session::MemoryBudget;
use ooxml_word_layout::facts::BlockKeeps;
use ooxml_word_layout::fields::update::PageNumbers;
use ooxml_word_layout::paint::drawings::{DrawingSlot, Layer};
use ooxml_word_layout::paint::{Drawn, GlyphSize, PaintedLine, Rule, Span};
use ooxml_word_layout::px::Px;
use ooxml_word_pages::{DiskFonts, EngineBudgets, EngineOptions, FontSetup, LaidVersion, PageEngine, PageView};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyTuple};
use std::sync::{Arc, Mutex, PoisonError};

pyo3::create_exception!(
    ooxml_core,
    LayoutFallback,
    pyo3::exceptions::PyException,
    "A version the Rust layout does not lay out as docx2svg does yet: args (reason, where)."
);

/// An exact length as `(numerator, denominator)`.
type Exact = (i128, i128);

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

fn exact(value: Px) -> Exact {
    let q = value.value();
    (*q.numer(), *q.denom())
}

/// A machine's fonts for one set of folders (Word's order) and options, scanned once and
/// shared by every document laid out with them.
#[pyclass(frozen, name = "WordFonts", module = "ooxml_core")]
pub struct PyWordFonts {
    inner: Arc<FontSetup>,
}

#[pymethods]
impl PyWordFonts {
    /// `folders`: `(location, path)` pairs, `location` `"bundle"` for Word's own bundle, else
    /// `"folder"`; `substitutes`: face (lowercased) -> the face used in its place.
    #[new]
    #[pyo3(signature = (folders, *, substitutes = None, recorded_faces = true, fallbacks = true))]
    fn new(
        py: Python<'_>,
        folders: Vec<(String, String)>,
        substitutes: Option<Vec<(String, String)>>,
        recorded_faces: bool,
        fallbacks: bool,
    ) -> Self {
        let folders: Vec<Folder> = folders
            .into_iter()
            .map(|(location, path)| Folder {
                location: if location == "bundle" {
                    Location::WordBundle
                } else {
                    Location::Folder
                },
                path,
            })
            .collect();
        let options = WordFontOptions {
            substitutes: substitutes.unwrap_or_else(|| WordFontOptions::default().substitutes),
            recorded_faces,
            fallbacks,
        };
        let inner = py.detach(|| Arc::new(FontSetup::new(Arc::new(DiskFonts), &folders, options)));
        Self { inner }
    }
}

/// One document's page engine: its versions laid out, each from the one before.
#[pyclass(frozen, name = "WordPages", module = "ooxml_core")]
pub struct PyWordPages {
    engine: Arc<Mutex<PageEngine>>,
}

fn lock(engine: &Mutex<PageEngine>) -> std::sync::MutexGuard<'_, PageEngine> {
    engine.lock().unwrap_or_else(PoisonError::into_inner)
}

type Laid = Result<Result<Arc<LaidVersion>, ooxml_word_pages::Fallback>, ooxml_word_pages::PagesError>;

fn laid_or_raise(engine: &Arc<Mutex<PageEngine>>, laid: Laid) -> PyResult<PyLaidPages> {
    match laid {
        Ok(Ok(version)) => Ok(PyLaidPages {
            version,
            engine: Arc::clone(engine),
        }),
        Ok(Err(fallback)) => Err(LayoutFallback::new_err((fallback.reason, fallback.at))),
        Err(error) => Err(PyValueError::new_err(error.to_string())),
    }
}

impl PyWordPages {
    /// The edit core's layout port over this engine (a call's `layout`).
    pub(crate) fn port(&self) -> ooxml_tools_word::EnginePages {
        ooxml_tools_word::EnginePages::new(Arc::clone(&self.engine))
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
        let (thousands, date_order, list_separator) = numbers;
        let numbers = ooxml_numfmt::Locale::new(decimal, thousands, &date_order, locale, list_separator)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let locale = Locale::new(locale).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let glyph_size = match glyph_size {
            "device" => GlyphSize::Device,
            "exact" => GlyphSize::Exact,
            other => return Err(PyValueError::new_err(format!("glyph_size {other:?}: device or exact"))),
        };
        let options = EngineOptions {
            locale,
            numbers,
            decimal,
            glyph_size,
            ..EngineOptions::default()
        };
        let budget = budget.map_or_else(|| MemoryBudget::new(OWN_BUDGET), |b| Arc::clone(&b.inner));
        let budgets = EngineBudgets {
            held: budget.holding(),
            paint: budget.account(),
        };
        Ok(Self {
            engine: Arc::new(Mutex::new(PageEngine::new(Arc::clone(&fonts.inner), options, budgets))),
        })
    }

    /// Lay out the version whose package is `data`, from the last version laid out: only
    /// the parts whose stored bytes changed are read again. Raises `LayoutFallback` for a
    /// version the Rust layout does not lay out as docx2svg does yet.
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
                                version.id_of(&l.path).map(str::to_owned),
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
        py.detach(|| paths.iter().map(|p| version.id_of(p).map(str::to_owned)).collect())
    }

    /// The body blocks each page draws something of (docx2svg's `Page.blocks`).
    fn blocks(&self, py: Python<'_>) -> Vec<Vec<String>> {
        let version = &self.version;
        py.detach(|| {
            version
                .facts()
                .pages
                .iter()
                .map(|p| p.blocks.iter().map(ToString::to_string).collect())
                .collect()
        })
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
            let numbers = PageNumbers::new(version.layout(), version.facts());
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
        Ok(lines
            .into_iter()
            .map(|l| {
                let (story, part) = l.story.map_or((None, None), |(k, p)| (Some(k), Some(p)));
                (
                    l.path,
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
    /// draws it.
    fn svg(&self, py: Python<'_>, page: usize) -> PyResult<(String, Vec<String>)> {
        let (version, engine) = (&self.version, &self.engine);
        py.detach(|| lock(engine).page_svg(version, page)).ok_or_else(|| {
            PyRuntimeError::new_err(format!(
                "page {page} not painted (0-based, of {})",
                version.page_count()
            ))
        })
    }
}

/// What `LaidPages.coverage` answers.
type CoverageRows = (Vec<GradeRow>, Vec<(String, char, String)>, Vec<String>);

/// A face's grade row (see `LaidPages.coverage`).
type GradeRow = (
    String,
    &'static str,
    Option<String>,
    bool,
    Option<&'static str>,
    Option<&'static str>,
);

fn grade_row(family: &str, grade: &ooxml_fonts::Grade) -> GradeRow {
    use ooxml_fonts::{Grade, SubstitutionKind};
    let plain = |name| (family.to_owned(), name, None, false, None, None);
    match grade {
        Grade::Installed => plain("installed"),
        Grade::Embedded => plain("embedded"),
        Grade::Recorded => plain("recorded"),
        Grade::Missing => plain("missing"),
        Grade::Substituted(s) => (
            family.to_owned(),
            "substituted",
            Some(s.substitute.clone()),
            s.same_metrics,
            Some(match s.kind {
                SubstitutionKind::Named => "named",
                SubstitutionKind::Default => "default",
                SubstitutionKind::Generic => "generic",
            }),
            s.class.map(ooxml_fonts::FaceClass::name),
        ),
    }
}

/// A page's frame row (see `LaidPages.pages`).
type PageRow = (
    (i64, usize, bool, &'static str, bool),
    (i64, i64, Exact, Exact, (Exact, Exact, Exact, Exact)),
);

fn page_row(info: &ooxml_word_layout::paginate::PageInfo, frame: &ooxml_word_layout::geometry::PageFrame) -> PageRow {
    let q = |v: ooxml_base::Q| (*v.numer(), *v.denom());
    (
        (
            info.number,
            info.section,
            info.section_first,
            info.story.name(),
            info.blank,
        ),
        (
            frame.width_px,
            frame.height_px,
            q(frame.width_pt),
            q(frame.height_pt),
            (
                exact(frame.left),
                exact(frame.right),
                exact(frame.top),
                exact(frame.bottom),
            ),
        ),
    )
}

fn slot_dict<'py>(py: Python<'py>, slot: &DrawingSlot) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("path", &slot.path)?;
    d.set_item("paragraph_path", &slot.paragraph_path)?;
    d.set_item("run", slot.run)?;
    d.set_item("index", slot.index)?;
    d.set_item("inline", slot.inline)?;
    d.set_item("x", exact(slot.x))?;
    d.set_item("y", exact(slot.y))?;
    d.set_item("width", exact(slot.width))?;
    d.set_item("height", exact(slot.height))?;
    d.set_item("behind", slot.layer == Layer::Behind)?;
    d.set_item("order", slot.order)?;
    d.set_item("story", slot.story.as_ref().map(|(k, p)| (*k, p.to_string())))?;
    Ok(d)
}

fn span_tuple<'py>(py: Python<'py>, s: &Span) -> PyResult<Bound<'py, PyTuple>> {
    let xs: Vec<Exact> = s.pens().map(exact).collect();
    let items: Vec<Bound<'py, PyAny>> = vec![
        s.path.as_str().into_pyobject(py)?.into_any(),
        s.chars.iter().collect::<String>().into_pyobject(py)?.into_any(),
        xs.into_pyobject(py)?.into_any(),
        s.y.into_pyobject(py)?.into_any(),
        exact(s.end).into_pyobject(py)?.into_any(),
        s.face.as_ref().into_pyobject(py)?.into_any(),
        s.bold.into_pyobject(py)?.to_owned().into_any(),
        s.italic.into_pyobject(py)?.to_owned().into_any(),
        s.half_points.into_pyobject(py)?.into_any(),
        s.color.as_deref().into_pyobject(py)?.into_any(),
        s.underline.as_deref().into_pyobject(py)?.into_any(),
        s.strike.into_pyobject(py)?.to_owned().into_any(),
        s.double_strike.into_pyobject(py)?.to_owned().into_any(),
        s.highlight.into_pyobject(py)?.into_any(),
        s.shading.as_deref().into_pyobject(py)?.into_any(),
        s.kind.name().into_pyobject(py)?.into_any(),
        s.line_baseline.into_pyobject(py)?.into_any(),
        s.vertical_align.as_deref().into_pyobject(py)?.into_any(),
    ];
    PyTuple::new(py, items)
}

fn line_tuple<'py>(py: Python<'py>, l: &PaintedLine) -> PyResult<Bound<'py, PyTuple>> {
    let spans = l
        .spans
        .iter()
        .map(|s| span_tuple(py, s))
        .collect::<PyResult<Vec<_>>>()?;
    let (story, part) = match &l.story {
        Some((kind, part)) => (Some(*kind), Some(part.to_string())),
        None => (None, None),
    };
    let items: Vec<Bound<'py, PyAny>> = vec![
        l.path.as_ref().into_pyobject(py)?.into_any(),
        l.number.into_pyobject(py)?.into_any(),
        l.baseline.into_pyobject(py)?.into_any(),
        exact(l.top).into_pyobject(py)?.into_any(),
        exact(l.pitch).into_pyobject(py)?.into_any(),
        l.paragraph_id.as_deref().into_pyobject(py)?.into_any(),
        l.band.into_pyobject(py)?.into_any(),
        story.into_pyobject(py)?.into_any(),
        part.into_pyobject(py)?.into_any(),
        l.column.into_pyobject(py)?.into_any(),
        spans.into_pyobject(py)?.into_any(),
    ];
    PyTuple::new(py, items)
}

fn rule_tuple(r: &Rule) -> RuleRow<'_> {
    (
        r.kind,
        r.path.as_str(),
        exact(r.x),
        exact(r.y),
        exact(r.width),
        exact(r.height),
        r.color.as_deref(),
        r.style.as_deref(),
    )
}

/// A rule's row (see `LaidPages.page`).
type RuleRow<'a> = (
    &'static str,
    &'a str,
    Exact,
    Exact,
    Exact,
    Exact,
    Option<&'a str>,
    Option<&'a str>,
);

type CropRow = Option<[Exact; 4]>;

fn picture_tuple(p: &Drawn) -> (&str, Option<&str>, Exact, Exact, Exact, Exact, Option<&str>, CropRow) {
    let crop = p.crop.map(|c| c.map(|q| (*q.numer(), *q.denom())));
    (
        p.path.as_str(),
        p.relationship.as_deref(),
        exact(p.x),
        exact(p.y),
        exact(p.width),
        exact(p.height),
        p.part.as_deref(),
        crop,
    )
}

fn view_dict<'py>(py: Python<'py>, view: &PageView, with_lines: bool) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("number", view.page + 1)?;
    d.set_item("info", page_row(&view.info, &view.frame))?;
    let lines = view
        .lines
        .iter()
        .filter(|_| with_lines)
        .map(|l| line_tuple(py, l))
        .collect::<PyResult<Vec<_>>>()?;
    d.set_item("lines", lines)?;
    d.set_item("rules", view.rules.iter().map(rule_tuple).collect::<Vec<_>>())?;
    d.set_item("pictures", view.pictures.iter().map(picture_tuple).collect::<Vec<_>>())?;
    let slots = view
        .drawings
        .iter()
        .zip(&view.drawing_faces)
        .zip(&view.text_box_lines)
        .map(|((s, faces), text)| {
            let d = slot_dict(py, s)?;
            d.set_item("faces", faces.clone())?;
            let lines = text
                .iter()
                .filter(|_| with_lines)
                .map(|l| line_tuple(py, l))
                .collect::<PyResult<Vec<_>>>()?;
            d.set_item("lines", lines)?;
            Ok(d)
        })
        .collect::<PyResult<Vec<_>>>()?;
    d.set_item("drawings", slots)?;
    d.set_item(
        "blocks",
        view.blocks.iter().map(ToString::to_string).collect::<Vec<_>>(),
    )?;
    d.set_item("unpainted", &view.unpainted)?;
    Ok(d)
}
