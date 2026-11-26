//! `SESSION_WRITERS_BENCH=<a.docx>[,<b.docx>...] cargo test --release -p ooxml-session --test
//! integration -- --ignored --nocapture writers_bench::`: several writers on one document at
//! once through a session, as a host's threads call it, in two phases on a document opened
//! afresh for each:
//! - `small`: writers each setting the text of their own paragraph (`word_set_text`) and a
//!   reader (`word_read`);
//! - `mixed`: the same, plus whole-document edits -- every field updated (`word_fields`
//!   update), the Normal style's spacing changed (`word_styles` modify), a paragraph put in a
//!   list and out again (`word_lists`), a comment added (`word_comments`) -- and a reader
//!   that checks the layout (`check`);
//! - `batch` (not in the default): the small writers and the reader beside a host's batch of
//!   [`BATCH_OPS`] text edits on one paragraph, run as one transaction that holds the
//!   document's gate from its open to its close (`Session::open_group`);
//!
//! The model's `batch` tool beside the same writers (Phase 3's optimistic batch) is timed through
//! the Rust host: ooxml-oracle's `host_batch_bench`.
//!
//! `SESSION_WRITERS_PHASES` (default `small,mixed`) names the phases; a phase named after one
//! whole-document worker (`fields`, `styles`, `lists`, `comments`, `check`) runs the small
//! writers and the reader with that one alone, to say which command holds the lock.
//!
//! Each phase prints, per tool, its calls, failures and wall times (p50, p99, max: what a
//! writer waits), then the store's commits, conflicts, exclusive runs, and how long its commit
//! lock was held and waited for each time it was taken (p50, p99, max), with the machine's
//! load before and after. `SESSION_WRITERS_SECS` (default 20) is how long each phase starts
//! calls; `SESSION_WRITERS_SMALL` (default 6) how many paragraph writers there are.
//!
//! Must not: run without the variable; it is a timing, not a test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{test_fonts, Memos, Never, NoFonts, NoParts, Unlimited};
use ooxml_base::context::Timestamp;
use ooxml_base::{Clock, Context, Limits};
use ooxml_opc::Package;
use ooxml_session::{CallContext, DocId, DocKind, MemoryBudget, Session, SessionConfig};
use ooxml_session_word::SessionPages;
use ooxml_store::{Caller, Histogram, StoreConfig};
use ooxml_tools_word::PageSetup;
use ooxml_word_edit::Editor;
use ooxml_word_pages::EngineOptions;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The system's clocks: the store times its lock with `monotonic`.
struct Real(Instant);

impl Clock for Real {
    fn now(&self) -> Timestamp {
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Timestamp {
            seconds: i64::try_from(since.as_secs()).unwrap_or(0),
            nanos: since.subsec_nanos(),
        }
    }
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

impl ooxml_store::Clock for Real {
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

fn session() -> Session {
    let config = SessionConfig::default();
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
    Session::new(config, context, ooxml_session_word::formats(pages))
}

/// The body's top-level paragraphs with lowercase text, `wanted` of them spread over the
/// document: each one's id and text.
fn paragraphs(bytes: &Arc<[u8]>, wanted: usize) -> Vec<(String, String)> {
    let package = Package::open(Arc::clone(bytes), &Limits::default()).unwrap();
    let editor = Editor::open(
        package,
        StoreConfig::default(),
        Arc::new(Real(Instant::now())),
        Arc::new(Memos),
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
        .filter(|(_, text)| text.len() > 20 && text.chars().any(char::is_lowercase))
        .collect();
    assert!(
        found.len() >= wanted,
        "{} paragraphs with text, {wanted} wanted",
        found.len()
    );
    let step = found.len() / wanted;
    (0..wanted).map(|k| found[k * step + step / 2].clone()).collect()
}

/// `text` with its first lowercase letter swapped, so that setting it changes the paragraph.
fn swapped(text: &str) -> String {
    let at = text.find(char::is_lowercase).unwrap();
    let with = if text[at..].starts_with('e') { "a" } else { "e" };
    format!("{}{with}{}", &text[..at], &text[at + 1..])
}

/// The machine's load averages (1, 5, 15 minutes), as the system says them.
fn load() -> String {
    if let Ok(text) = std::fs::read_to_string("/proc/loadavg") {
        return text.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
    }
    std::process::Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// What one tool's calls took.
#[derive(Default)]
struct Calls {
    times: Vec<Duration>,
    failed: usize,
    first_error: Option<String>,
}

type Book = Arc<Mutex<BTreeMap<String, Calls>>>;

/// One call, timed and booked under `label`.
fn call(s: &Session, book: &Book, label: &str, tool: &str, arguments: Value, context: &CallContext) {
    let started = Instant::now();
    let result = s.call(tool, arguments, context);
    let took = started.elapsed();
    let mut book = book.lock().unwrap();
    let calls = book.entry(label.to_owned()).or_default();
    calls.times.push(took);
    if result.is_error() {
        calls.failed += 1;
        if calls.first_error.is_none() {
            let reply = Value::Object(result.to_json());
            let text = reply.get("error").map(ToString::to_string).unwrap_or_default();
            calls.first_error = Some(text.chars().take(300).collect());
        }
    }
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1000.0)
}

fn exact(times: &mut [Duration], q: f64) -> Duration {
    times.sort();
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let rank = ((q * times.len() as f64).ceil() as usize).clamp(1, times.len());
    times[rank - 1]
}

fn histogram(name: &str, h: &Histogram) -> String {
    format!(
        "{name}: {} times, p50 {}, p99 {}, max {}, total {}",
        h.count(),
        ms(h.percentile(0.5)),
        ms(h.percentile(0.99)),
        ms(h.max()),
        ms(h.total())
    )
}

/// A writer or reader thread's call, its round given.
type Work = Box<dyn Fn(&Session, &DocId, &Book, usize) + Send + Sync>;

/// A thread: a whole-document one calls until the phase's time is up; the others go on
/// until every whole-document one has stopped too, so that its longest commit has writers
/// beside it.
struct Worker {
    label: String,
    whole: bool,
    work: Work,
}

fn run_phase(name: &str, bytes: &Arc<[u8]>, workers: Vec<Worker>, secs: u64, stamp: Option<&(String, String)>) {
    let s = Arc::new(session());
    let opened = Instant::now();
    let d = s.open(Arc::clone(bytes), DocKind::Docx, "bench.docx").unwrap();
    let open = opened.elapsed();
    // laid out once before the writers start, as a host's first check does
    let warmed = Instant::now();
    let first = s.call(
        "check",
        json!({"doc": d.to_string(), "include": ["layout"]}),
        &CallContext::default(),
    );
    assert!(!first.is_error(), "check: {:?}", first.to_json());
    let warm = warmed.elapsed();
    // every block stamped with an id before the writers start, as the first edit of a
    // document does (and the edits after it no more)
    if let Some((id, text)) = stamp {
        for set in [swapped(text), text.clone()] {
            let done = s.call(
                "word_set_text",
                json!({"doc": d.to_string(), "target": id, "text": set}),
                &CallContext::default(),
            );
            assert!(!done.is_error(), "stamping: {:?}", done.to_json());
        }
    }
    let book: Book = Arc::default();
    let before = load();
    let started = Instant::now();
    let until = started + Duration::from_secs(secs);
    let wholes = Arc::new(AtomicUsize::new(workers.iter().filter(|w| w.whole).count()));
    let handles: Vec<_> = workers
        .into_iter()
        .map(|Worker { label, whole, work }| {
            let (s, d, book, wholes) = (Arc::clone(&s), d.clone(), Arc::clone(&book), Arc::clone(&wholes));
            std::thread::Builder::new()
                .name(label)
                .spawn(move || {
                    let mut round = 0;
                    while Instant::now() < until || (!whole && wholes.load(Ordering::SeqCst) > 0) {
                        work(&s, &d, &book, round);
                        round += 1;
                    }
                    if whole {
                        wholes.fetch_sub(1, Ordering::SeqCst);
                    }
                })
                .unwrap()
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let wall = started.elapsed();
    let after = load();
    let stats = s.store_stats(&d).unwrap();
    let times = s.lock_times(&d).unwrap();
    println!(
        "== phase {name}: open {}, first check {}; load before {before}, after {after}",
        ms(open),
        ms(warm)
    );
    print_calls(&mut book.lock().unwrap());
    #[allow(clippy::cast_precision_loss)]
    let rate = stats.commits as f64 / wall.as_secs_f64();
    println!(
        "  store: {} commits in {:.1} s ({rate:.1}/s), {} conflicts, {} rebased, {} exclusive",
        stats.commits,
        wall.as_secs_f64(),
        stats.conflicts,
        stats.rebased,
        stats.exclusive
    );
    println!("  lock {}", histogram("held", &times.held));
    println!("  lock {}", histogram("waited", &times.waited));
    println!("  {}", histogram("gate held", &times.gated));
    println!("  {}", histogram("gate waited", &times.gate_waited));
}

/// Each tool's calls, failures and wall times.
fn print_calls(book: &mut BTreeMap<String, Calls>) {
    for (label, calls) in book.iter_mut() {
        let n = calls.times.len();
        let (p50, p99, max) = (
            exact(&mut calls.times, 0.5),
            exact(&mut calls.times, 0.99),
            exact(&mut calls.times, 1.0),
        );
        println!(
            "  {label}: {n} calls ({} failed), p50 {}, p99 {}, max {}",
            calls.failed,
            ms(p50),
            ms(p99),
            ms(max)
        );
        if let Some(error) = &calls.first_error {
            println!("    first error: {error}");
        }
    }
}

fn context(name: &str) -> CallContext {
    CallContext {
        caller: Caller::named(name),
        ..CallContext::default()
    }
}

/// `small` writers, each setting its own paragraph's text back and forth.
fn small_writers(targets: &[(String, String)]) -> Vec<Worker> {
    targets
        .iter()
        .enumerate()
        .map(|(k, (id, text))| {
            let (id, text, other) = (id.clone(), text.clone(), swapped(text));
            let label = format!("set_text#{k}");
            let caller = context(&label);
            let work: Work = Box::new(move |s, d, book, round| {
                let set = if round % 2 == 0 { &other } else { &text };
                let arguments = json!({"doc": d.to_string(), "target": id, "text": set});
                call(s, book, "word_set_text", "word_set_text", arguments, &caller);
            });
            Worker {
                label,
                whole: false,
                work,
            }
        })
        .collect()
}

fn reader() -> Worker {
    let caller = context("reader");
    let work: Work = Box::new(move |s, d, book, _| {
        call(
            s,
            book,
            "word_read",
            "word_read",
            json!({"doc": d.to_string()}),
            &caller,
        );
    });
    Worker {
        label: "reader".to_owned(),
        whole: false,
        work,
    }
}

/// How many ops the `batch` phase's batch holds.
const BATCH_OPS: usize = 20;

/// A host's batch: [`BATCH_OPS`] text edits of one paragraph in one step group, closed as
/// one step; timed whole (open to close).
fn batch_writer(target: (String, String)) -> Worker {
    let caller = context("batch");
    let (id, text) = target;
    let other = swapped(&text);
    let work: Work = Box::new(move |s, d, book, _| {
        let started = Instant::now();
        let group = s.open_group(d).unwrap();
        let mut failed = 0;
        for op in 0..BATCH_OPS {
            let set = if op % 2 == 0 { &other } else { &text };
            let arguments = json!({"doc": d.to_string(), "target": id, "text": set});
            failed += usize::from(s.call("word_set_text", arguments, &caller).is_error());
        }
        s.close_group(d, &group, &caller, Some("batch")).unwrap();
        let mut book = book.lock().unwrap();
        let calls = book.entry(format!("batch of {BATCH_OPS}")).or_default();
        calls.times.push(started.elapsed());
        calls.failed += failed;
    });
    Worker {
        label: "batch".to_owned(),
        whole: true,
        work,
    }
}

fn whole_document(list: (String, String), comment: (String, String)) -> Vec<Worker> {
    let fields: Work = {
        let caller = context("fields");
        Box::new(move |s, d, book, _| {
            let arguments = json!({"doc": d.to_string(), "action": "update"});
            call(s, book, "word_fields update", "word_fields", arguments, &caller);
        })
    };
    let styles: Work = {
        let caller = context("styles");
        Box::new(move |s, d, book, round| {
            let after = if round % 2 == 0 { 7.0 } else { 8.0 };
            let arguments = json!({"doc": d.to_string(), "action": "modify", "name": "Normal", "space_after": after});
            call(s, book, "word_styles modify", "word_styles", arguments, &caller);
        })
    };
    let lists: Work = {
        let caller = context("lists");
        let id = list.0;
        Box::new(move |s, d, book, round| {
            let action = if round % 2 == 0 { "add" } else { "remove" };
            let arguments = json!({"doc": d.to_string(), "targets": [id], "action": action});
            call(
                s,
                book,
                &format!("word_lists {action}"),
                "word_lists",
                arguments,
                &caller,
            );
        })
    };
    let comments: Work = {
        let caller = context("comments");
        let id = comment.0;
        Box::new(move |s, d, book, round| {
            let arguments = json!({"doc": d.to_string(), "action": "add",
                "items": [{"target": id, "text": format!("Comment {round}")}]});
            call(s, book, "word_comments add", "word_comments", arguments, &caller);
        })
    };
    let check: Work = {
        let caller = context("check");
        Box::new(move |s, d, book, _| {
            let arguments = json!({"doc": d.to_string(), "include": ["layout"]});
            call(s, book, "check", "check", arguments, &caller);
        })
    };
    [
        ("fields", fields),
        ("styles", styles),
        ("lists", lists),
        ("comments", comments),
        ("check", check),
    ]
    .into_iter()
    .map(|(label, work)| Worker {
        label: label.to_owned(),
        whole: true,
        work,
    })
    .collect()
}

#[test]
#[ignore = "a timing of concurrent writers: SESSION_WRITERS_BENCH=<docx> cargo test --release -- --ignored"]
fn mixed_writers_on_one_document() {
    let Ok(paths) = std::env::var("SESSION_WRITERS_BENCH") else {
        return;
    };
    let secs = std::env::var("SESSION_WRITERS_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let small = std::env::var("SESSION_WRITERS_SMALL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6usize);
    let phases = std::env::var("SESSION_WRITERS_PHASES").unwrap_or_else(|_| "small,mixed".to_owned());
    for path in paths.split(',') {
        let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
        println!("# {path}: {small} paragraph writers, {secs} s a phase");
        let mut targets = paragraphs(&bytes, small + 2);
        let comment = targets.pop().unwrap();
        let list = targets.pop().unwrap();
        for phase in phases.split(',') {
            let mut workers = small_writers(&targets);
            workers.push(reader());
            let whole = whole_document(list.clone(), comment.clone());
            match phase {
                "small" => {}
                "mixed" => workers.extend(whole),
                "batch" => workers.push(batch_writer(list.clone())),
                one => workers.extend(whole.into_iter().filter(|w| w.label == one)),
            }
            run_phase(phase, &bytes, workers, secs, targets.first());
        }
    }
}
