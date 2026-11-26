//! `SESSION_CHECK_BENCH=<path to a .docx> cargo test --release -p ooxml-session --test
//! integration -- --ignored --nocapture check_bench::`: a long document checked through a
//! session as a host does it -- opened once with the session's own page engine, a check of
//! its layout, one letter of a middle paragraph's text set (`word_set_text`), a check
//! again, then that paragraph made several lines longer and a check again -- each call's
//! wall time.
//!
//! Must not: run without the variable; it is a timing, not a test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::test_fonts;
use crate::locale::session_with;
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_session::{CallContext, DocKind, MemoryBudget, Session, SessionConfig};
use ooxml_session_word::prelude::*;
use ooxml_session_word::SessionPages;
use ooxml_store::StoreConfig;
use ooxml_tools_word::{PageSetup, WordLayout};
use ooxml_word_edit::Editor;
use ooxml_word_pages::EngineOptions;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The middle top-level paragraph of the body with lowercase text: its id and its text.
fn middle(bytes: &Arc<[u8]>) -> (String, String) {
    let package = Package::open(Arc::clone(bytes), &Limits::default()).unwrap();
    let editor = Editor::open(
        package,
        StoreConfig::default(),
        Arc::new(Monotonic(Instant::now())),
        Arc::new(crate::common::Memos),
    )
    .unwrap();
    let docx = editor.store().snapshot().doc().docx().unwrap();
    let body = docx.body().unwrap();
    let texts = body.texts();
    let found: Vec<(String, String)> = body
        .ids
        .paragraphs()
        .iter()
        .enumerate()
        .filter(|(_, p)| p.path.len() == 2)
        .filter_map(|(k, p)| Some((p.id.clone(), texts.get(k)?.to_owned())))
        .filter(|(_, text)| text.chars().any(char::is_lowercase))
        .collect();
    found[found.len() / 2].clone()
}

struct Monotonic(Instant);

impl ooxml_store::Clock for Monotonic {
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

fn timed(s: &Session, tool: &str, arguments: Value) -> (Value, Duration) {
    let started = Instant::now();
    let reply = Value::Object(s.call(tool, arguments, &CallContext::default()).to_json());
    let took = started.elapsed();
    assert!(reply.get("error").is_none(), "{tool}: {reply}");
    (reply, took)
}

#[test]
#[ignore = "a timing on a long document: SESSION_CHECK_BENCH=<docx> cargo test --release -- --ignored"]
fn checks_after_edits_on_a_long_document() {
    let Ok(path) = std::env::var("SESSION_CHECK_BENCH") else {
        return;
    };
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let (id, text) = middle(&bytes);
    let s = session_with(
        SessionConfig::default(),
        Some(SessionPages {
            setup: PageSetup::new(test_fonts(), EngineOptions::default()),
            memory: MemoryBudget::new(4 << 30),
            raster: None,
        }),
    );
    let started = Instant::now();
    let d = s.open(Arc::clone(&bytes), DocKind::Docx, "report.docx").unwrap();
    let open = started.elapsed();
    let check = json!({"doc": d.to_string(), "include": ["layout"]});
    let (_, first) = timed(&s, "check", check.clone());
    let at = text.find(char::is_lowercase).unwrap();
    let swapped = if text[at..].starts_with('e') { "a" } else { "e" };
    let letter = format!("{}{swapped}{}", &text[..at], &text[at + 1..]);
    let (_, edit) = timed(
        &s,
        "word_set_text",
        json!({"doc": d.to_string(), "target": id, "text": letter}),
    );
    let (_, again) = timed(&s, "check", check.clone());
    let longer = format!(
        "{text} {}",
        "Several more words to make this paragraph a few lines longer. ".repeat(6)
    );
    let (_, grow) = timed(
        &s,
        "word_set_text",
        json!({"doc": d.to_string(), "target": id, "text": longer}),
    );
    let (_, moved) = timed(&s, "check", check);
    let tier = s.pages(&d).unwrap().unwrap().engine().lock().unwrap().page_tier();
    println!(
        "page tier: {} pages, {} KB ({} B a page) on the paint account",
        tier.entries,
        tier.bytes >> 10,
        tier.bytes / tier.entries.max(1)
    );
    let (judged, bytes) = s.last_check(&d).unwrap().unwrap().language_judgements();
    println!(
        "language judgements: {judged} paragraphs, {} KB on the document's account",
        bytes >> 10
    );
    let ms = |d: Duration| format!("{:.1} ms", d.as_secs_f64() * 1000.0);
    println!(
        "open {}; first check {}; set one letter {}; check {}; set a longer text {}; check {}",
        ms(open),
        ms(first),
        ms(edit),
        ms(again),
        ms(grow),
        ms(moved)
    );
}
