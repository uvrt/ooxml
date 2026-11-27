//! `SESSION_FEED_BENCH=<path to a .docx> cargo test --release -p ooxml-session-word --test
//! integration -- --ignored --nocapture feed_bench::`: a long document's live view through a
//! session as a host reads it -- every page first, then after one letter set, a paragraph made
//! several lines longer (a reflow), a paragraph inserted before an early one, and an undo:
//! read as today's live view reads it (the view since the one before, then each changed page's
//! SVG) and as the changes feed sends it, each on a session of its own
//! (`SESSION_FEED_READERS=view,feed`) -- each call's wall time, the pages it lists, the bytes
//! sent and the engine's digest counts.
//!
//! Must not: run without the variable; it is a timing, not a test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{test_fonts, Never, NoFonts, NoParts, Unlimited};
use ooxml_base::{Context, Limits};
use ooxml_opc::Package;
use ooxml_session::{CallContext, DocId, DocKind, MemoryBudget, Session, SessionConfig};
use ooxml_session_word::prelude::*;
use ooxml_session_word::{SessionPages, TextMode};
use ooxml_store::StoreConfig;
use ooxml_tools_word::{PageSetup, WordLayout};
use ooxml_word_edit::Editor;
use ooxml_word_pages::EngineOptions;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Monotonic(Instant);

impl ooxml_store::Clock for Monotonic {
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

struct Real(Instant);

impl ooxml_base::Clock for Real {
    fn now(&self) -> ooxml_base::context::Timestamp {
        ooxml_base::context::Timestamp {
            seconds: 1_795_000_000,
            nanos: 0,
        }
    }
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

/// The body's top-level paragraphs with lowercase text: each one's id and text.
fn paragraphs(bytes: &Arc<[u8]>) -> Vec<(String, String)> {
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
    body.ids
        .paragraphs()
        .iter()
        .enumerate()
        .filter(|(_, p)| p.path.len() == 2)
        .filter_map(|(k, p)| Some((p.id.clone(), texts.get(k)?.to_owned())))
        .filter(|(_, text)| text.chars().any(char::is_lowercase))
        .collect()
}

fn session() -> Session {
    let pages = Some(SessionPages {
        setup: PageSetup::new(test_fonts(), EngineOptions::default()),
        memory: MemoryBudget::new(4 << 30),
        raster: None,
    });
    let context = Context {
        fonts: Arc::new(NoFonts),
        parts: Arc::new(NoParts),
        clock: Arc::new(Real(Instant::now())),
        budget: Arc::new(Unlimited),
        cancel: Arc::new(Never),
        limits: Limits::default(),
        locale: ooxml_base::Locale::default(),
    };
    Session::new(SessionConfig::default(), context, ooxml_session_word::formats(pages))
}

fn edit(s: &Session, tool: &str, arguments: Value) -> Duration {
    let started = Instant::now();
    let reply = Value::Object(s.call(tool, arguments, &CallContext::default()).to_json());
    let took = started.elapsed();
    assert!(reply.get("error").is_none(), "{tool}: {reply}");
    took
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

/// How a reader of the pages reads them after each step.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reader {
    /// The view since the one before, then each changed page's SVG (today's live view).
    View,
    /// The changes feed since the one before, with pages.
    Feed,
}

/// The pages read since `since` as `reader` reads them, timed and printed under `label`;
/// the next `since`.
fn read(s: &Session, d: &DocId, since: Option<&str>, reader: Reader, label: &str) -> String {
    let pages = s.pages(d).unwrap().unwrap();
    let digested = || {
        let stats = pages.engine().lock().unwrap().stats();
        (stats.pages_digested, stats.full, stats.relaid)
    };
    let (before, full, relaid) = digested();
    let context = CallContext::default();
    let started = Instant::now();
    let (token, listed, bytes, groups) = match reader {
        Reader::View => {
            let view = s.view(d, since, &*pages, &context).unwrap();
            let mut bytes = 0;
            for page in &view.changed {
                bytes += s
                    .page_svg(d, *page, &view.token, &*pages, TextMode::Text, &context)
                    .unwrap()
                    .svg
                    .len();
            }
            (view.token, view.changed, bytes, 0)
        }
        Reader::Feed => {
            let changes = s.changes(d, since, true, &*pages, &context).unwrap();
            let found = changes.pages.unwrap();
            let mut bytes = found.list.as_ref().map_or(0, |l| l.len() * 48);
            let mut groups = 0;
            for page in &found.changed {
                groups += page.groups.len() + page.moved.len();
                bytes += page.groups.iter().map(|g| g.id.len() + g.svg.len() + 12).sum::<usize>();
                bytes += page.moved.iter().map(|(id, _)| id.len() + 12).sum::<usize>();
                bytes += page.removed.iter().map(|id| id.len() + 3).sum::<usize>();
                bytes += page.defs.iter().map(|(id, d)| id.len() + d.len()).sum::<usize>();
                bytes += page.order.as_ref().map_or(0, |o| o.iter().map(|id| id.len() + 3).sum());
                bytes += page.pictures.iter().map(|p| p.bytes.len() * 4 / 3 + 40).sum::<usize>();
            }
            let listed = found.changed.iter().map(|p| p.page).collect();
            (changes.token, listed, bytes, groups)
        }
    };
    let took = started.elapsed();
    let (after, full_after, relaid_after) = digested();
    let shown: Vec<usize> = listed.iter().copied().take(8).collect();
    let what = if reader == Reader::View {
        "view+page_svg"
    } else {
        "changes"
    };
    println!(
        "{label}: {what} {} ({} pages {shown:?}, {groups} groups, {:.1} KB; digested {}, taken {}; laid out whole {}, again {})",
        ms(took),
        listed.len(),
        bytes as f64 / 1024.0,
        after.0 - before.0,
        after.1 - before.1,
        full_after - full,
        relaid_after - relaid,
    );
    token
}

/// The steps, read after each as `reader` reads them, on a session of its own.
fn steps(bytes: &Arc<[u8]>, reader: Reader) {
    let found = paragraphs(bytes);
    let (middle, text) = found[found.len() / 2].clone();
    let (early, _) = found[found.len().min(40) / 4].clone();
    let s = session();
    let d = s.open(Arc::clone(bytes), DocKind::Docx, "report.docx").unwrap();
    let doc = d.to_string();
    let token = read(&s, &d, None, reader, "first");
    let at = text.find(char::is_lowercase).unwrap();
    let swapped = if text[at..].starts_with('e') { "a" } else { "e" };
    let letter = format!("{}{swapped}{}", &text[..at], &text[at + 1..]);
    let took = edit(
        &s,
        "word_set_text",
        json!({"doc": doc, "target": middle, "text": letter}),
    );
    println!("one letter: edit {}", ms(took));
    let token = read(&s, &d, Some(&token), reader, "one letter");
    let longer = format!(
        "{letter} {}",
        "Several more words to make this paragraph a few lines longer. ".repeat(6)
    );
    let took = edit(
        &s,
        "word_set_text",
        json!({"doc": doc, "target": middle, "text": longer}),
    );
    println!("reflow: edit {}", ms(took));
    let token = read(&s, &d, Some(&token), reader, "reflow");
    let took = edit(
        &s,
        "word_insert_markdown",
        json!({"doc": doc, "at": format!("before:{early}"), "markdown": "An inserted paragraph."}),
    );
    println!("insert: edit {}", ms(took));
    let token = read(&s, &d, Some(&token), reader, "insert");
    let took = edit(&s, "undo", json!({"doc": doc}));
    println!("undo: edit {}", ms(took));
    read(&s, &d, Some(&token), reader, "undo");
}

#[test]
#[ignore = "a timing on a long document: SESSION_FEED_BENCH=<docx> cargo test --release -- --ignored"]
fn live_view_after_edits_on_a_long_document() {
    let Ok(path) = std::env::var("SESSION_FEED_BENCH") else {
        return;
    };
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let readers = std::env::var("SESSION_FEED_READERS").unwrap_or_else(|_| "view,feed".to_owned());
    for reader in readers.split(',') {
        match reader {
            "view" => steps(&bytes, Reader::View),
            "feed" => steps(&bytes, Reader::Feed),
            other => panic!("no reader {other}"),
        }
    }
}
