//! Owns: [`EnginePages`], the edit core's [`FieldLayout`] port over a Word page engine
//! (`ooxml-word-pages`): a version laid out from the one the engine laid out last, its page
//! numbers read as a field update reads them (`ooxml_word_layout::fields::update`), places
//! named by the tool's block ids (each found from its id when asked).
//!
//! The host that keeps a document's engine (its fonts, its budgets) hands it in; the
//! engine is locked while it lays out a version, never while a page number is read.
//!
//! Must not: hold anything but the engine, or lay out but through it.

use ooxml_opc::Package;
use ooxml_word_edit::{FieldLayout, FieldPlace, LaidFields, PageShows, Unknown};
use ooxml_word_layout::facts::LayoutFacts;
use ooxml_word_layout::fields::update::PageNumbers;
use ooxml_word_model::value::Text;
use ooxml_word_pages::{LaidVersion, PageEngine};
use std::borrow::Borrow;
use std::sync::{Arc, Mutex, PoisonError};

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

impl FieldLayout for EnginePages {
    fn lay_out(&self, package: &Package) -> Result<Box<dyn LaidFields + '_>, String> {
        let laid = {
            let mut engine = self.engine.lock().unwrap_or_else(PoisonError::into_inner);
            engine.lay_out(package)
        };
        match laid {
            Ok(Ok(version)) => Ok(Box::new(Laid::new(version))),
            Ok(Err(fallback)) => Err(format!("{} at {}", fallback.reason, fallback.at)),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// A laid version's facts, held for its page numbers.
struct Facts(Arc<LaidVersion>);

impl Borrow<LayoutFacts> for Facts {
    fn borrow(&self) -> &LayoutFacts {
        self.0.facts()
    }
}

/// One version laid out: its page numbers, its blocks' paths found by id.
struct Laid {
    version: Arc<LaidVersion>,
    numbers: PageNumbers<Facts>,
}

impl Laid {
    fn new(version: Arc<LaidVersion>) -> Laid {
        let numbers = PageNumbers::new(version.layout(), Facts(Arc::clone(&version)));
        Laid { version, numbers }
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

impl LaidFields for Laid {
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
