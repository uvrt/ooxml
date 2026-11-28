//! `FIELDS_BENCH=<path to a .docx> cargo test --release -p ooxml-tools-word --test integration -- --ignored
//! --nocapture fields_bench::`: the field tools on a long document, each the best of three on a fresh editor
//! (opening not counted; a call whose run takes over [`ONCE_PAST`] runs once, to keep the gates' bench within
//! its 30-minute limit), over one page engine kept for the document as a host keeps it (its first layout timed
//! apart): before each call the engine has laid out the version the call starts from, as a host's kept layout
//! has, and the fields have been listed once (the document's field scans, as a session keeps them).
//! `FIELDS_CALLS=footer,one` (names: update, again, one, toc, footer) runs only those; `again` (asked by name
//! only, to keep the gates' bench within its limit) is an update of every field after one
//! has updated them (not counted).
//!
//! Must not: run without the variable; it is a timing, not a test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{engine, Monotonic, Never, Unlimited};
use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::{ToolArgs, ToolResult};
use ooxml_tools_word::{word_fields, word_headers_footers, CallEditor, Editing, EnginePages, WordFields};
use ooxml_word_edit::{Editor, FieldLayout, LaidFields, LayOutError};
use ooxml_word_pages::PageEngine;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The port, its layouts counted and timed (what a call spends laying out).
struct Timed {
    engine: Arc<Mutex<PageEngine>>,
    inner: EnginePages,
    spent: Mutex<Vec<Duration>>,
}

impl FieldLayout for Timed {
    fn lay_out(
        &self,
        version: &Arc<ooxml_word_model::docx::Docx>,
        cancel: &dyn Cancel,
    ) -> Result<Box<dyn LaidFields + '_>, LayOutError> {
        let started = Instant::now();
        let laid = self.inner.lay_out(version, cancel);
        self.spent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(started.elapsed());
        laid
    }
}

impl Timed {
    /// The layouts since the last call, as "n layouts, x ms".
    fn take(&self) -> String {
        let spent = std::mem::take(&mut *self.spent.lock().unwrap_or_else(PoisonError::into_inner));
        let each: Vec<String> = spent
            .iter()
            .map(|d| format!("{:.0}", d.as_secs_f64() * 1000.0))
            .collect();
        format!("{} layouts [{}] ms", spent.len(), each.join(", "))
    }
}

/// A run longer than this is not repeated: its best of three would not change the picture and
/// three such runs of each slow call pass the gates' 30-minute limit.
const ONCE_PAST: Duration = Duration::from_secs(20);

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

/// The best of three calls of `tool` (one past [`ONCE_PAST`]) with `arguments`, each on a fresh editor whose opened
/// version the engine has laid out (not counted), and the last reply.
fn best<A: ToolArgs>(
    bytes: &Arc<[u8]>,
    layout: &Timed,
    arguments: &Value,
    tool: impl Fn(&A, &Editing<'_>) -> ToolResult<Value>,
) -> (Duration, Value) {
    best_after(bytes, layout, None, arguments, tool)
}

/// As [`best`], each call after `first` (an update) has run on the editor, not counted.
fn best_after<A: ToolArgs>(
    bytes: &Arc<[u8]>,
    layout: &Timed,
    first: Option<&Value>,
    arguments: &Value,
    tool: impl Fn(&A, &Editing<'_>) -> ToolResult<Value>,
) -> (Duration, Value) {
    let mut out = (Duration::MAX, Value::Null);
    for _ in 0..3 {
        let editor = editor(bytes);
        let opened = editor.store().snapshot().doc().clone();
        let warm = layout.engine.lock().unwrap().lay_out(&opened.docx().unwrap(), None);
        assert!(matches!(warm, Ok(Ok(_))), "the opened version is laid out");
        let caller = Caller::anonymous();
        let call_editor = CallEditor::new(&editor);
        let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z")
            .with_layout(Some(layout));
        // The document's field scans as a session keeps them between calls (not counted).
        let (list, _) = WordFields::accept(json!({"doc": "d1", "action": "list"}), Some("docx")).unwrap();
        assert!(word_fields(&list, &editing).to_json().contains_key("data"));
        if let Some(first) = first {
            let kept = layout.spent.lock().unwrap().len();
            let (update, _) = WordFields::accept(first.clone(), Some("docx")).unwrap();
            assert!(word_fields(&update, &editing).to_json().contains_key("summary"));
            layout.spent.lock().unwrap().truncate(kept);
        }
        let before = layout.engine.lock().unwrap().stats();
        let (args, _) = A::accept(arguments.clone(), Some("docx")).unwrap();
        let started = Instant::now();
        let reply = tool(&args, &editing);
        let took = started.elapsed();
        let after = layout.engine.lock().unwrap().stats();
        println!(
            "  run {:.1} ms: engine open {:.1} ms, layout {:.1} ms, {} relaid, {} whole",
            took.as_secs_f64() * 1000.0,
            (after.open_time - before.open_time).as_secs_f64() * 1000.0,
            (after.layout_time - before.layout_time).as_secs_f64() * 1000.0,
            after.relaid - before.relaid,
            after.full - before.full,
        );
        if took < out.0 {
            out = (took, Value::Object(reply.to_json()));
        }
        if took > ONCE_PAST {
            break;
        }
    }
    out
}

#[test]
#[ignore = "a timing on a long document: FIELDS_BENCH=<docx> cargo test --release -- --ignored"]
fn field_tools_on_a_long_document() {
    let Ok(path) = std::env::var("FIELDS_BENCH") else {
        return;
    };
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let probe = editor(&bytes);
    let docx = probe.store().snapshot().doc().docx().unwrap();
    let ids: Vec<String> = docx
        .body()
        .unwrap()
        .ids
        .paragraphs()
        .iter()
        .filter(|p| p.path.len() == 2)
        .map(|p| p.id.clone())
        .collect();
    let engine = Arc::new(Mutex::new(engine()));
    let started = Instant::now();
    let first = engine
        .lock()
        .unwrap()
        .lay_out(&probe.store().snapshot().doc().docx().unwrap(), None);
    let pages = first.unwrap().map(|v| v.page_count());
    println!(
        "{} body paragraphs; first layout {:.1} ms ({pages:?} pages)",
        ids.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    let seed = engine.lock().unwrap().seed();
    let layout = Timed {
        inner: EnginePages::new(Arc::clone(&engine), seed),
        engine,
        spent: Mutex::new(Vec::new()),
    };
    run_calls(&bytes, &layout, &ids);
}

/// The calls `FIELDS_CALLS` names (every one by default), each reported.
fn run_calls(bytes: &Arc<[u8]>, layout: &Timed, ids: &[String]) {
    let report = |name: &str, (took, reply): (Duration, Value)| {
        let summary = reply.get("summary").cloned().unwrap_or(Value::Null);
        let warnings = reply.get("warnings").cloned().unwrap_or(Value::Null);
        println!(
            "{name}: {:.1} ms -- {summary} {warnings}; over the runs {}",
            took.as_secs_f64() * 1000.0,
            layout.take()
        );
    };
    let calls = std::env::var("FIELDS_CALLS").unwrap_or_else(|_| "update,one,toc,footer".to_owned());
    let wanted = |name: &str| calls.split(',').any(|c| c == name);
    if wanted("update") {
        report(
            "word_fields update",
            best(bytes, layout, &json!({"doc": "d1", "action": "update"}), word_fields),
        );
    }
    if wanted("again") {
        let update = json!({"doc": "d1", "action": "update"});
        report(
            "word_fields update again",
            best_after(bytes, layout, Some(&update), &update, word_fields),
        );
    }
    if wanted("one") {
        let field = page_field(bytes, layout);
        report(
            &format!("word_fields update {field}"),
            best(
                bytes,
                layout,
                &json!({"doc": "d1", "action": "update", "ids": [field]}),
                word_fields,
            ),
        );
    }
    if wanted("toc") {
        report(
            "word_fields insert_toc",
            best(
                bytes,
                layout,
                &json!({"doc": "d1", "action": "insert_toc", "after": ids[10]}),
                word_fields,
            ),
        );
    }
    if wanted("footer") {
        report(
            "word_headers_footers page_x_of_y",
            best(
                bytes,
                layout,
                &json!({"doc": "d1", "section": "s:1", "which": "footer", "action": "set", "text": "",
                        "page_number": "page_x_of_y"}),
                word_headers_footers,
            ),
        );
    }
}

/// The id of a field whose result the pages say (a PAGEREF, else a PAGE, else the first
/// field), from `word_fields list`.
fn page_field(bytes: &Arc<[u8]>, layout: &Timed) -> String {
    let editor = editor(bytes);
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let call_editor = CallEditor::new(&editor);
    let editing =
        Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z").with_layout(Some(layout));
    let (args, _) = <WordFields as ToolArgs>::accept(json!({"doc": "d1", "action": "list"}), Some("docx")).unwrap();
    let listed = Value::Object(word_fields(&args, &editing).to_json());
    let fields = listed["data"].as_array().cloned().unwrap_or_default();
    let keyword = |k: &str| fields.iter().find(|f| f["keyword"] == k);
    let field = keyword("PAGEREF")
        .or_else(|| keyword("PAGE"))
        .or_else(|| fields.first())
        .expect("a field");
    println!("single field: {}", field);
    field["id"].as_str().unwrap().to_owned()
}
