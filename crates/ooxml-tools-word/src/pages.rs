//! Owns: [`EnginePages`], the edit core's [`FieldLayout`] port over a Word page engine
//! (`ooxml-word-pages`): a version laid out from the one the engine laid out last, its page
//! numbers read as a field update reads them (`ooxml_word_layout::fields::update`), places
//! named by the tool's block ids (each found from its id when asked), the pages a block is
//! drawn on (`LayoutFacts::pages_of_each`), a `STYLEREF`'s text read from the page painted
//! (the engine paints it when asked), a character's advance in the version's fonts; and the
//! reflow between two versions, each summarised as docx-agent's `LayoutSummary` is
//! (`ooxml_word_pages::summary`).
//!
//! The host that keeps a document's engine (its fonts, its budgets) hands it in; the
//! engine is locked while it lays out a version or paints a page, never while a page number
//! is read.
//!
//! Also owns [`WordLayout`], the port Word's tools are handed a document's layout through:
//! the edit core's [`FieldLayout`], and the page engine behind it for what a tool reads of
//! the pages themselves (`check`'s reflow and layout facts).
//!
//! Must not: hold anything but the engine, or lay out but through it.

use ooxml_base::Cancel;
use ooxml_opc::Package;
use ooxml_word_edit::{
    CharAdvance, FieldLayout, FieldPlace, FieldRun, LaidFields, LayOutError, PageReflow, PageShows, Unknown,
};
use ooxml_word_layout::facts::LayoutFacts;
use ooxml_word_layout::fields::update::{drawn_text, style_reference_known, PageNumbers};
use ooxml_word_model::value::Text;
use ooxml_word_pages::summary::Summary;
use ooxml_word_pages::{Fallback, LaidVersion, PageEngine, PagesError};
use std::borrow::Borrow;
use std::sync::{Arc, Mutex, PoisonError};

/// The host's layout of a document's versions, as Word's tools are handed it: the edit
/// core's port ([`FieldLayout`], what an edit reads of the pages), and the document's page
/// engine, for a tool that reads the pages themselves (`check`).
pub trait WordLayout: FieldLayout {
    /// The document's page engine: locked while a version is laid out or a page painted.
    fn engine(&self) -> &Arc<Mutex<PageEngine>>;

    /// `package` (a version of the document) laid out by [`WordLayout::engine`], stopped
    /// where `cancel` says so.
    ///
    /// # Errors
    /// Why it was not laid out: stopped, or a version the engine does not lay out yet (in
    /// its words).
    fn laid(&self, package: &Package, cancel: &dyn Cancel) -> Result<Arc<LaidVersion>, NotLaid> {
        let mut engine = self.engine().lock().unwrap_or_else(PoisonError::into_inner);
        match engine.lay_out_until(package, cancel) {
            Err(PagesError::Stopped) => Err(NotLaid::Stopped),
            found => laid(found).map_err(NotLaid::Other),
        }
    }
}

/// How long laying a version out may take in a call (docx-agent's
/// `Limits.layout_timeout`), counted from the call's start: past it, what reads the layout
/// says so (`check`'s `reflow.error`) and the call goes on.
#[derive(Clone, Copy)]
pub struct LayoutLimit<'a> {
    /// Says stop where the call is cancelled or the limit passed.
    pub cancel: &'a dyn Cancel,
    /// The limit, as said where it passed.
    pub timeout: std::time::Duration,
}

/// Why [`WordLayout::laid`] laid no version out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotLaid {
    /// Its `cancel` stopped it.
    Stopped,
    /// The engine does not lay it out (in its words).
    Other(String),
}

/// The edit core's layout port over a page engine.
pub struct EnginePages {
    engine: Arc<Mutex<PageEngine>>,
}

impl std::fmt::Debug for EnginePages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnginePages").finish_non_exhaustive()
    }
}

impl EnginePages {
    /// The port over `engine` (one document's).
    #[must_use]
    pub fn new(engine: Arc<Mutex<PageEngine>>) -> EnginePages {
        EnginePages { engine }
    }
}

/// A version laid out, or why not, in the port's words.
fn laid(found: Result<Result<Arc<LaidVersion>, Fallback>, PagesError>) -> Result<Arc<LaidVersion>, String> {
    match found {
        Ok(Ok(version)) => Ok(version),
        Ok(Err(fallback)) => Err(format!("{} at {}", fallback.reason, fallback.at)),
        Err(error) => Err(error.to_string()),
    }
}

/// `package` laid out, stopped where `cancel` says so, and its pages summarised as
/// docx-agent compares two layouts (`DocumentLayout.compare`: every line of text a page
/// draws, its text boxes' too).
fn summarised(engine: &mut PageEngine, package: &Package, cancel: &dyn Cancel) -> Result<Summary, LayOutError> {
    let found = engine.lay_out_until(package, cancel);
    if matches!(found, Err(PagesError::Stopped)) {
        return Err(LayOutError::Stopped);
    }
    let version = laid(found).map_err(LayOutError::Unsupported)?;
    engine
        .contents(&version)
        .ok_or_else(|| LayOutError::Unsupported("the version laid out is not the engine's".to_owned()))
}

impl WordLayout for EnginePages {
    fn engine(&self) -> &Arc<Mutex<PageEngine>> {
        &self.engine
    }
}

impl FieldLayout for EnginePages {
    fn lay_out(&self, package: &Package, cancel: &dyn Cancel) -> Result<Box<dyn LaidFields + '_>, LayOutError> {
        let found = {
            let mut engine = self.engine.lock().unwrap_or_else(PoisonError::into_inner);
            engine.lay_out_until(package, cancel)
        };
        if matches!(found, Err(PagesError::Stopped)) {
            return Err(LayOutError::Stopped);
        }
        let version = laid(found).map_err(LayOutError::Unsupported)?;
        Ok(Box::new(Laid::new(version, &self.engine)))
    }

    fn reflow(&self, before: &Package, after: &Package, cancel: &dyn Cancel) -> Result<PageReflow, LayOutError> {
        let (was, now) = {
            let mut engine = self.engine.lock().unwrap_or_else(PoisonError::into_inner);
            (
                summarised(&mut engine, before, cancel)?,
                summarised(&mut engine, after, cancel)?,
            )
        };
        let found = now.compare(&was, &std::collections::HashMap::new());
        Ok(PageReflow {
            changed: found.changed,
            page_count: found.page_count,
            moved: found.moved,
            unknown: found.unknown,
        })
    }
}

/// A laid version's facts, held for its page numbers.
struct Facts(Arc<LaidVersion>);

impl Borrow<LayoutFacts> for Facts {
    fn borrow(&self) -> &LayoutFacts {
        self.0.facts()
    }
}

/// One version laid out: its page numbers, its blocks' paths found by id, and the engine
/// that paints its pages.
struct Laid<'e> {
    version: Arc<LaidVersion>,
    numbers: PageNumbers<Facts>,
    engine: &'e Mutex<PageEngine>,
}

impl<'e> Laid<'e> {
    fn new(version: Arc<LaidVersion>, engine: &'e Mutex<PageEngine>) -> Laid<'e> {
        let numbers = PageNumbers::new(version.layout(), Facts(Arc::clone(&version)));
        Laid {
            version,
            numbers,
            engine,
        }
    }

    fn path(&self, id: &str) -> Result<Text, Unknown> {
        self.version
            .path_of(id)
            .ok_or_else(|| Unknown(format!("{id} is not laid out")))
    }
}

fn unknown(error: ooxml_word_layout::fields::update::Unknown) -> Unknown {
    Unknown(error.0)
}

impl LaidFields for Laid<'_> {
    fn page_field(&self, instruction: &str, keyword: &str, at: FieldPlace<'_>) -> Result<String, Unknown> {
        match at {
            FieldPlace::Body(id) => self
                .numbers
                .page_field(instruction, keyword, &self.path(id)?)
                .map_err(unknown),
            FieldPlace::Page(page) => self.numbers.page_field_at(instruction, keyword, page).map_err(unknown),
        }
    }

    fn page_reference(&self, instruction: &str, at: FieldPlace<'_>) -> Result<String, Unknown> {
        match at {
            FieldPlace::Body(id) => self
                .numbers
                .page_reference(instruction, &self.path(id)?)
                .map_err(unknown),
            FieldPlace::Page(page) => Err(Unknown(format!("a reference to page {page}"))),
        }
    }

    fn style_reference(&self, instruction: &str, at: FieldPlace<'_>, run: FieldRun<'_>) -> Result<String, Unknown> {
        let (page, story) = match at {
            FieldPlace::Body(id) => {
                let path = self.path(id)?;
                let page = self
                    .numbers
                    .drawn_on(&path)
                    .ok_or_else(|| Unknown(format!("{id} is not drawn")))?;
                (page, None)
            }
            FieldPlace::Page(page) => (page, Some(run.part)),
        };
        let mut engine = self.engine.lock().unwrap_or_else(PoisonError::into_inner);
        style_reference_known(&self.version.layout().document, engine.locale(), instruction).map_err(unknown)?;
        let painted = engine
            .paint(&self.version, page)
            .ok_or_else(|| Unknown(format!("page {page} is not painted")))?;
        Ok(drawn_text(&painted, story, run.path))
    }

    fn blocks_pages(&self, ids: &[&str]) -> Vec<Result<Vec<usize>, Unknown>> {
        let paths: Vec<Option<Text>> = ids.iter().map(|id| self.version.path_of(id)).collect();
        let keys: Vec<&str> = paths.iter().map(|p| p.as_deref().unwrap_or_default()).collect();
        let found = self.version.facts().pages_of_each(&keys);
        ids.iter()
            .zip(paths.iter().zip(found))
            .map(|(id, (path, pages))| match path {
                Some(_) if !pages.is_empty() => Ok(pages),
                Some(_) => Err(Unknown(format!("{id} is not drawn: the layout stops before it"))),
                None => Err(Unknown(format!("{id} is not laid out"))),
            })
            .collect()
    }

    /// The engine's fonts for this version, then the measured tables of six faces (docx2svg's
    /// `TableAdvances`, which docx-agent's `_TocLooks._width` asks after the installed fonts).
    fn advance(&self, family: &str, bold: bool, italic: bool, ch: char) -> Option<CharAdvance> {
        let (advance, units_per_em) = self
            .version
            .advance(family, bold, italic, ch)
            .or_else(|| ooxml_fonts::table_advance(family, bold, italic, ch))?;
        Some(CharAdvance { advance, units_per_em })
    }

    fn pages(&self) -> Vec<PageShows> {
        self.numbers
            .pages()
            .map(|info| PageShows {
                section: info.section,
                kind: info.story.name(),
                blank: info.blank,
            })
            .collect()
    }
}
