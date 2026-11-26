//! `HOST_BATCH_BENCH=<a.docx> cargo test --release -p ooxml-oracle --test integration --
//! --ignored --nocapture host_batch_bench::`: the model's `batch` tool through the Rust host
//! beside writers (design phase 3, section 7.6; #385's `batch` phase, now the host's optimistic
//! batch): [`WRITERS`] writers (`HOST_BATCH_WRITERS`: 0 times the batch alone) each setting their own paragraph's text (`word_set_text`), a
//! reader (`word_read`), and a batch of [`BATCH_OPS`] text edits of one paragraph, back to back,
//! all through one `HostSession` of Word's format, for `HOST_BATCH_SECS` (default 8). Prints
//! each tool's calls and wall times (p50, p99, max) -- a batch with a failed op counted as
//! failed -- and the store's commits, conflicts, rebases and exclusive runs.
//!
//! Must not: run without the variable; it is a timing, not a test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_host::{CallAs, HostSession};
use ooxml_oracle::host_replay::Replayer;
use ooxml_session::{DocId, DocKind};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The paragraph writers beside the batch.
const WRITERS: usize = 6;

/// The ops of each batch.
const BATCH_OPS: usize = 20;

type Book = Arc<Mutex<BTreeMap<String, (Vec<Duration>, usize)>>>;

fn caller(name: &str) -> CallAs {
    let mut context = Map::new();
    context.insert("agent".into(), Value::from(name));
    CallAs {
        context: Some(context),
        ..CallAs::default()
    }
}

/// The paragraphs whose ids and text a `word_read` of the document names, `wanted` of them
/// spread over it (lowercase text of some length, top level).
fn paragraphs(s: &HostSession, wanted: usize) -> Vec<(String, String)> {
    // the whole document, page by page of the read
    let mut text = String::new();
    let mut cursor = Value::Null;
    loop {
        let read = s
            .call("word_read", json!({"doc": "d1", "cursor": cursor}), &CallAs::default())
            .unwrap();
        let read = Value::Object(read.to_json());
        text.push_str(read["data"].as_str().unwrap_or(""));
        text.push('\n');
        cursor = read["next_cursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    let mut found = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(id) = line
            .strip_prefix("<!-- ")
            .and_then(|l| l.strip_suffix(" -->"))
            .and_then(|l| l.split_whitespace().next())
        else {
            continue;
        };
        // (by id, or by place where the document's blocks have no ids yet)
        if let (true, Some(next)) = (id.starts_with("p:") || id.starts_with("p@body/"), lines.peek()) {
            if next.len() > 20 && next.chars().all(|c| c.is_alphanumeric() || " ,.;:'-()".contains(c)) {
                found.push((id.to_owned(), (*next).to_owned()));
            }
        }
    }
    assert!(found.len() >= wanted, "{} paragraphs, {wanted} wanted", found.len());
    let step = found.len() / wanted;
    (0..wanted).map(|k| found[k * step + step / 2].clone()).collect()
}

fn swapped(text: &str) -> String {
    let at = text.find(char::is_lowercase).unwrap();
    let with = if text[at..].starts_with('e') { "a" } else { "e" };
    format!("{}{with}{}", &text[..at], &text[at + 1..])
}

fn note(book: &Book, label: &str, took: Duration, failed: bool) {
    let mut book = book.lock().unwrap();
    let entry = book.entry(label.to_owned()).or_default();
    entry.0.push(took);
    entry.1 += usize::from(failed);
}

fn quantile(times: &mut [Duration], q: f64) -> Duration {
    times.sort_unstable();
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let at = ((times.len() as f64 - 1.0) * q).round() as usize;
    times.get(at).copied().unwrap_or_default()
}

#[test]
#[ignore = "a timing of the host's batch beside writers: HOST_BATCH_BENCH=<docx> cargo test --release -- --ignored"]
fn a_batch_beside_writers_through_the_host() {
    let Ok(path) = std::env::var("HOST_BATCH_BENCH") else {
        return;
    };
    let secs: u64 = std::env::var("HOST_BATCH_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let writers: usize = std::env::var("HOST_BATCH_WRITERS")
        .ok()
        .and_then(|w| w.parse().ok())
        .unwrap_or(WRITERS);
    let s = opened(&path, writers);
    let d = DocId::nth(1);
    let targets = paragraphs(&s, writers + 1);
    let book: Book = Arc::default();
    let format = s.session().format_doc(&d).unwrap();
    let before = format.stored().stats();
    let until = Instant::now() + Duration::from_secs(secs);
    let mut handles = Vec::new();
    for (k, (id, text)) in targets.iter().enumerate() {
        let (s, book, id, text) = (Arc::clone(&s), Arc::clone(&book), id.clone(), text.clone());
        let batch = k == writers;
        handles.push(std::thread::spawn(move || {
            let how = caller(&format!("w{k}"));
            let mut round = 0usize;
            while Instant::now() < until {
                let set = if round.is_multiple_of(2) {
                    swapped(&text)
                } else {
                    text.clone()
                };
                let (label, tool, arguments) = if batch {
                    let ops: Vec<Value> = (0..BATCH_OPS)
                        .map(|op| {
                            let set = if op.is_multiple_of(2) {
                                swapped(&text)
                            } else {
                                text.clone()
                            };
                            json!({"tool": "word_set_text", "arguments": {"doc": "d1", "target": id, "text": set}})
                        })
                        .collect();
                    (format!("batch of {BATCH_OPS}"), "batch", json!({"ops": ops}))
                } else {
                    let arguments = json!({"doc": "d1", "target": id, "text": set});
                    ("word_set_text".to_owned(), "word_set_text", arguments)
                };
                let started = Instant::now();
                let result = s.call(tool, arguments, &how).unwrap();
                // a batch whose op failed is no fair timing: counted as failed
                let ops_failed = batch
                    && Value::Object(result.to_json())["data"]["failed"]
                        .as_array()
                        .is_some_and(|f| !f.is_empty());
                note(&book, &label, started.elapsed(), result.is_error() || ops_failed);
                if result.is_error() && round < 2 {
                    eprintln!("{label} failed: {}", Value::Object(result.to_json()));
                }
                round += 1;
            }
        }));
    }
    {
        let (s, book) = (Arc::clone(&s), Arc::clone(&book));
        handles.push(std::thread::spawn(move || {
            while Instant::now() < until {
                let started = Instant::now();
                let result = s.call("word_read", json!({"doc": "d1"}), &CallAs::default()).unwrap();
                note(&book, "word_read", started.elapsed(), result.is_error());
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    let after = format.stored().stats();
    println!("# {path}: {writers} writers, a reader and a batch of {BATCH_OPS}, {secs} s");
    report(&book, &before, &after);
}

/// A host session of Word's format with the document at `path` open as `d1`, laid out once
/// and every block stamped with an id, as a host's first check and first edit leave it.
fn opened(path: &str, writers: usize) -> Arc<HostSession> {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let replayer = Replayer::new(root).unwrap();
    let s = Arc::new(replayer.session(&json!({"session": {}})).unwrap());
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let d = s.open(bytes, DocKind::Docx, "bench.docx").unwrap();
    assert_eq!(d, DocId::nth(1));
    let first = s
        .call("check", json!({"doc": "d1", "include": ["layout"]}), &CallAs::default())
        .unwrap();
    assert!(!first.is_error(), "check: {:?}", first.to_json());
    let targets = paragraphs(&s, writers + 1);
    // every block stamped with an id before the writers start, as the first edit of a
    // document does (and the edits after it no more): as the session's writers bench
    if let Some((id, text)) = targets.first() {
        for set in [swapped(text), text.clone()] {
            let done = s
                .call(
                    "word_set_text",
                    json!({"doc": "d1", "target": id, "text": set}),
                    &CallAs::default(),
                )
                .unwrap();
            assert!(!done.is_error(), "stamping: {:?}", done.to_json());
        }
    }
    s
}

/// Each tool's calls and wall times, and what the store met.
fn report(book: &Book, before: &ooxml_store::StoreStats, after: &ooxml_store::StoreStats) {
    let ms = |d: Duration| format!("{:.1} ms", d.as_secs_f64() * 1000.0);
    for (label, (times, failed)) in book.lock().unwrap().iter_mut() {
        let n = times.len();
        let (p50, p99, max) = (quantile(times, 0.5), quantile(times, 0.99), quantile(times, 1.0));
        println!(
            "  {label}: {n} calls ({failed} failed), p50 {}, p99 {}, max {}",
            ms(p50),
            ms(p99),
            ms(max)
        );
    }
    println!(
        "  store: {} commits, {} conflicts, {} rebased, {} exclusive",
        after.commits - before.commits,
        after.conflicts - before.conflicts,
        after.rebased - before.rebased,
        after.exclusive - before.exclusive
    );
}
