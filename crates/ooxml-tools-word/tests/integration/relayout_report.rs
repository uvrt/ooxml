//! `RELAYOUT_REPORT=<path to a .docx> cargo test --release -p ooxml-tools-word --test integration --
//! --ignored --nocapture relayout_report::`: edits that move every later page of a long report -- a
//! paragraph inserted on page 1, a table of contents inserted, every field updated -- each version
//! laid out again by the engine that laid out the one before (timed) is the version laid out whole
//! in a fresh engine: each flow item alike (its lines broken alike), the pages, the fields' values
//! and the facts carried from the version before; then a check of its layout and page 1's SVG,
//! timed. `RELAYOUT_CALLS=insert,toc,update` runs only those.
//!
//! Must not: run without the variable; it is a timing and a long document's equality, not a
//! unit test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::check::checked;
use crate::common::{engine, Monotonic, Never, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_fields, CallEditor, Editing, EnginePages, LastCheck, WordFields};
use ooxml_word_edit::{Editor, InsertParagraph, RevisionPolicy};
use ooxml_word_layout::facts::LayoutFacts;
use ooxml_word_layout::incremental::same_item;
use ooxml_word_pages::{LaidVersion, PageEngine};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn editor(bytes: &Arc<[u8]>) -> Editor {
    let package = Package::open(Arc::clone(bytes), &Limits::default()).unwrap();
    Editor::open(
        package,
        StoreConfig::default(),
        Arc::new(Monotonic(Instant::now())),
        Arc::new(Unlimited),
    )
    .unwrap()
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn laid(engine: &Mutex<PageEngine>, package: &Package) -> (Arc<LaidVersion>, Duration) {
    let started = Instant::now();
    let version = engine.lock().unwrap().lay_out(package).unwrap().unwrap();
    (version, started.elapsed())
}

/// `ours` (laid out again) is `whole` (laid out whole in a fresh engine).
fn same_layout(name: &str, ours: &LaidVersion, whole: &LaidVersion) {
    let (a, b) = (ours.layout(), whole.layout());
    assert_eq!(a.flow.items.len(), b.flow.items.len(), "{name}: flow items");
    for (k, (x, y)) in a.flow.items.iter().zip(&b.flow.items).enumerate() {
        let alike = match (x.para(), y.para()) {
            (Some(_), Some(_)) => same_item(x, y),
            _ => std::mem::discriminant(x) == std::mem::discriminant(y),
        };
        assert!(alike && x.block() == y.block(), "{name}: flow item {k} differs");
    }
    assert!(a.pagination == b.pagination, "{name}: the pages differ");
    assert_eq!(a.fields, b.fields, "{name}: the fields' values differ");
    assert!(*ours.facts() == LayoutFacts::of(b), "{name}: the facts carried differ");
    assert_eq!(
        ours.facts().first_pages(),
        LayoutFacts::of(b).first_pages(),
        "{name}: the first pages carried differ"
    );
}

/// The version `edit` leaves, the engine having laid out the opened one; checked against a
/// whole layout, then checked and page 1 drawn, each timed.
fn run(name: &str, bytes: &Arc<[u8]>, edit: &dyn Fn(&Editor, &Arc<Mutex<PageEngine>>) -> Duration) {
    let kept = Arc::new(Mutex::new(engine()));
    let editor = editor(bytes);
    let opened = editor.store().snapshot().doc().package().clone();
    let (_, first) = laid(&kept, &opened);
    let edited = edit(&editor, &kept);
    let package = editor.store().snapshot().doc().package().clone();
    let (ours, again) = laid(&kept, &package);
    let fresh = Mutex::new(engine());
    let (whole, full) = laid(&fresh, &package);
    same_layout(name, &ours, &whole);
    let started = Instant::now();
    let pages = EnginePages::new(Arc::clone(&kept), kept.lock().unwrap().seed());
    let reply = checked(
        &package,
        2,
        json!({"doc": "d1", "include": ["layout"]}),
        &LastCheck::default(),
        Some(&pages),
    );
    let check = started.elapsed();
    assert!(reply.get("summary").is_some(), "{name}: {reply}");
    let started = Instant::now();
    let svg = kept.lock().unwrap().page_svg(&ours, 0).unwrap();
    let render = started.elapsed();
    assert!(svg.0.starts_with("<svg"), "{name}");
    println!(
        "{name}: {} pages; opened laid out {}; edit {}; laid out again {} (whole {}); check {}; page 1 drawn {}",
        ours.page_count(),
        ms(first),
        ms(edited),
        ms(again),
        ms(full),
        ms(check),
        ms(render),
    );
}

/// A field tool's call `arguments` on `editor`, over the engine (it lays out as it goes).
fn fields_call(editor: &Editor, kept: &Arc<Mutex<PageEngine>>, arguments: serde_json::Value) -> Duration {
    let opened = editor.store().snapshot().doc().clone();
    let pages = EnginePages::new(Arc::clone(kept), kept.lock().unwrap().seed());
    let caller = Caller::anonymous();
    let call_editor = CallEditor::new(editor);
    let editing =
        Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z").with_layout(Some(&pages));
    let (args, _) = WordFields::accept(arguments, Some("docx")).unwrap();
    let started = Instant::now();
    let reply = word_fields(&args, &editing).to_json();
    let took = started.elapsed();
    assert!(reply.contains_key("summary"), "{reply:?}");
    took
}

#[test]
#[ignore = "a long document's relayout: RELAYOUT_REPORT=<docx> cargo test --release -- --ignored"]
fn edits_that_move_every_later_page() {
    let Ok(path) = std::env::var("RELAYOUT_REPORT") else {
        return;
    };
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let ids: Vec<String> = {
        let probe = editor(&bytes);
        let docx = probe.store().snapshot().doc().docx().unwrap();
        let body = docx.body().unwrap();
        body.ids
            .paragraphs()
            .iter()
            .filter(|p| p.path.len() == 2)
            .map(|p| p.id.clone())
            .collect()
    };
    let calls = std::env::var("RELAYOUT_CALLS").unwrap_or_else(|_| "insert,toc,update".to_owned());
    let wanted = |name: &str| calls.split(',').any(|c| c == name);
    if wanted("insert") {
        run("a paragraph inserted on page 1", &bytes, &|editor, _| {
            let command = InsertParagraph {
                anchor: ids[2].clone(),
                after: true,
                text: "A paragraph inserted near the start, long enough to take two lines of the page's text \
                       column, so that every page after it starts elsewhere."
                    .to_owned(),
                style: None,
                policy: RevisionPolicy::Untracked,
            };
            let started = Instant::now();
            editor.run(&Caller::anonymous(), &command).unwrap();
            started.elapsed()
        });
    }
    if wanted("toc") {
        run("a table of contents inserted", &bytes, &|editor, kept| {
            fields_call(
                editor,
                kept,
                json!({"doc": "d1", "action": "insert_toc", "after": ids[10]}),
            )
        });
    }
    if wanted("update") {
        run("every field updated", &bytes, &|editor, kept| {
            fields_call(editor, kept, json!({"doc": "d1", "action": "update"}))
        });
    }
}
