//! Helpers the test modules share: clocks, an unlimited budget, no cancelling, the page
//! engine on the repository's fonts and the namespaces made-up parts declare (one copy instead
//! of one per module).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use ooxml_base::Cancel;
use ooxml_derive::CacheBudget;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_store::Clock;
use ooxml_word_pages::{DiskFonts, EngineBudgets, EngineOptions, FontSetup, PageEngine};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

pub struct Still;

impl Clock for Still {
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

pub struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _bytes: usize) -> bool {
        true
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

pub struct Monotonic(pub Instant);

impl Clock for Monotonic {
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

pub fn engine() -> PageEngine {
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
        held: Arc::new(Unlimited),
        paint: Arc::new(Unlimited),
    };
    PageEngine::new(setup, EngineOptions::default(), budgets)
}

pub const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;

pub const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
