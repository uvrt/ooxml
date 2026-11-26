//! A timing of `word_template` on a long document (lane P4d5): the document put in
//! compatibility mode 14, its head laid out first (the layout a session keeps), then Convert
//! with its reflow through the layout port, best of 3. Run with
//! `TEMPLATE_BENCH=<docx> cargo test --release -p ooxml-tools-word --test integration -- --ignored --nocapture template_bench::`.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{engine, Monotonic, Never, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_template, CallEditor, Editing, EnginePages, WordTemplate};
use ooxml_word_edit::Editor;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    let layout = EnginePages::new(Arc::clone(&engine), engine.lock().unwrap().seed());
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
        let first = engine.lock().unwrap().lay_out(opened.package(), None).unwrap().unwrap();
        println!(
            "head laid out: {} pages, {:.0} ms",
            first.page_count(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        let caller = Caller::anonymous();
        let call_editor = CallEditor::new(&editor);
        let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z")
            .with_layout(Some(&layout));
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
