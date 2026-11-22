//! Helpers the test modules share: a context with no fonts, no stored parts, no budget limit
//! and no cancelling, and a page engine over the repository's test fonts (one copy instead of
//! one per module).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use ooxml_base::context::{FaceData, FaceRequest};
use ooxml_base::{BaseError, CacheBudget, Cancel, FontSource, PartStore};
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_tools_word::EnginePages;
use ooxml_word_pages::{DiskFonts, EngineBudgets, EngineOptions, FontSetup, PageEngine};
use std::sync::Arc;

pub struct NoFonts;

impl FontSource for NoFonts {
    fn face(&self, _: &FaceRequest) -> Result<Option<FaceData>, BaseError> {
        Ok(None)
    }
}

pub struct NoParts;

impl PartStore for NoParts {
    fn load(&self, _: &str) -> Result<Option<Arc<[u8]>>, BaseError> {
        Ok(None)
    }
    fn store(&self, _: &str, _: Arc<[u8]>) -> Result<(), BaseError> {
        Ok(())
    }
}

pub struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _: usize) -> bool {
        true
    }
    fn release(&self, _: usize) {}
    fn used(&self) -> usize {
        0
    }
}

pub struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// A memo budget that grants every reservation (the page engine's).
pub struct Memos;

impl ooxml_derive::CacheBudget for Memos {
    fn reserve(&self, _: usize) -> bool {
        true
    }
    fn release(&self, _: usize) {}
    fn used(&self) -> usize {
        0
    }
}

/// The page engine a host lays its documents out with (the repository's test fonts), its
/// memos and paint within no limit.
pub fn layout() -> Arc<EnginePages> {
    let folder = Folder {
        location: Location::Folder,
        path: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../python/pptx2svg/packages/pptx2svg-fonts/src/pptx2svg_fonts/files"
        )
        .to_owned(),
    };
    let options = WordFontOptions {
        substitutes: Vec::new(),
        recorded_faces: true,
        fallbacks: true,
    };
    let setup = Arc::new(FontSetup::new(Arc::new(DiskFonts), &[folder], options));
    let budgets = EngineBudgets {
        held: Arc::new(Memos),
        paint: Arc::new(Memos),
    };
    let engine = PageEngine::new(setup, EngineOptions::default(), budgets);
    Arc::new(EnginePages::of(engine))
}
