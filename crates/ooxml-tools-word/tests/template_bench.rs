//! A timing of `word_template` on a long document (lane P4d5): the document put in
//! compatibility mode 14, its head laid out first (the layout a session keeps), then Convert
//! with its reflow through the layout port, best of 3. Run with
//! `TEMPLATE_BENCH=<docx> cargo test --release -p ooxml-tools-word --test template_bench -- --ignored --nocapture`.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::{Cancel, Limits};
use ooxml_derive::CacheBudget;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_template, CallEditor, Editing, EnginePages, WordTemplate};
use ooxml_word_edit::Editor;
use ooxml_word_pages::{DiskFonts, EngineBudgets, EngineOptions, FontSetup, PageEngine};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Never;
impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

struct Monotonic(Instant);
impl Clock for Monotonic {
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

struct Unlimited;
impl CacheBudget for Unlimited {
    fn reserve(&self, _bytes: usize) -> bool {
        true
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

fn engine() -> PageEngine {
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

/// The document of `bytes` in compatibility mode 14.
fn legacy(bytes: &Arc<[u8]>) -> Package {
    let package = Package::open(Arc::clone(bytes), &Limits::default()).unwrap();
    let settings = String::from_utf8(package.read("word/settings.xml").unwrap().unwrap().to_vec()).unwrap();
    let old = settings.replace(
        r#"w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15""#,
        r#"w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="14""#,
    );
    assert_ne!(old, settings, "the document is in mode 15");
    package.with_bytes("word/settings.xml", Arc::from(old.into_bytes()))
}

#[test]
#[ignore = "a timing on a long document: TEMPLATE_BENCH=<docx> cargo test --release -- --ignored"]
fn convert_on_a_long_document() {
    let Ok(path) = std::env::var("TEMPLATE_BENCH") else {
        return;
    };
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let engine = Arc::new(Mutex::new(engine()));
    let layout = EnginePages::new(Arc::clone(&engine));
    let (args, _) = WordTemplate::accept(json!({"doc": "d1", "action": "upgrade_to_modern"}), Some("docx")).unwrap();
    let mut best = (Duration::MAX, Value::Null);
    for _ in 0..3 {
        let editor = Editor::open(
            legacy(&bytes),
            StoreConfig::default(),
            Arc::new(Monotonic(Instant::now())),
            Arc::new(Unlimited),
        )
        .unwrap();
        let opened = editor.store().snapshot().doc().clone();
        let started = Instant::now();
        let first = engine.lock().unwrap().lay_out(opened.package()).unwrap().unwrap();
        println!(
            "head laid out: {} pages, {:.0} ms",
            first.page_count(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        let caller = Caller::anonymous();
        let editing = Editing {
            editor: &CallEditor::new(&editor),
            caller: &caller,
            opened: &opened,
            tracking: None,
            doc: "d1",
            cancel: &Never,
            date: "2026-11-17T10:00:00Z",
            layout: Some(&layout),
            others: ooxml_tools_word::Others::default(),
            blobs: None,
        };
        let started = Instant::now();
        let reply = Value::Object(word_template(&args, &editing).to_json());
        let took = started.elapsed();
        println!("word_template: {:.0} ms", took.as_secs_f64() * 1000.0);
        if took < best.0 {
            best = (took, reply);
        }
    }
    let reflow = &best.1["data"]["reflow"];
    println!(
        "word_template best of 3: {:.0} ms; pages {}, {} changed",
        best.0.as_secs_f64() * 1000.0,
        reflow["pages"],
        reflow["changed"].as_array().map_or(0, Vec::len)
    );
}
