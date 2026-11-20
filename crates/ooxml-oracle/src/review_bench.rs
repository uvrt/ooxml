//! `word-review-bench <document.docx> [changes]`: what the review tools cost on a long
//! document. The document is opened, `changes` tracked `set_text`s by two authors are
//! made through the edit core across the body (as docx-agent would write them), then, best
//! of three each on that version: `word_changes` list (grouped and as records), preview
//! of everything, accept all and reject all -- each a whole tool call, wall time -- as
//! JSON. CPU for the whole run is the caller's to measure (`/usr/bin/time -l`).
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::{Monotonic, Unlimited};
use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ArgType;
use ooxml_tools_word::{word_changes, CallEditor, Editing, WordChanges};
use ooxml_word_edit::{Editor, RevisionPolicy, SetText, Tracking, WordDoc};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

fn ms(d: std::time::Duration) -> f64 {
    (d.as_secs_f64() * 1e6).round() / 1e3
}

/// A fresh editor whose first version is `doc`.
fn editor_of(doc: &WordDoc) -> Result<Editor, String> {
    Editor::open(
        doc.package().clone(),
        StoreConfig::default(),
        Arc::new(Monotonic::new()),
        Arc::new(Unlimited),
    )
    .map_err(|e| e.to_string())
}

/// The best of three runs of a `word_changes` call on fresh editors of `doc`: its wall
/// time in ms and its summary.
fn best(doc: &WordDoc, arguments: &Value) -> Result<(f64, String), String> {
    let args = WordChanges::from_arg(arguments, "").map_err(|e| e.to_string())?;
    let mut times = Vec::new();
    let mut summary = String::new();
    for _ in 0..3 {
        let editor = editor_of(doc)?;
        let opened = editor.store().snapshot().doc().clone();
        let caller = Caller::anonymous();
        let editing = Editing {
            editor: &CallEditor::new(&editor),
            caller: &caller,
            opened: &opened,
            tracking: None,
            doc: "d1",
            cancel: &Never,
            date: "2026-11-17T10:00:00Z",
            layout: None,
            others: ooxml_tools_word::Others::default(),
            blobs: None,
            written: &[],
        };
        let started = Instant::now();
        let reply = word_changes(&args, &editing);
        times.push(ms(started.elapsed()));
        summary = reply
            .to_json()
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
    }
    times.sort_by(f64::total_cmp);
    Ok((times.first().copied().unwrap_or(0.0), summary))
}

/// Run over the arguments: one JSON object on stdout.
///
/// # Errors
/// A message when the document cannot be read.
pub fn run(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .ok_or("usage: word-review-bench <document.docx> [changes]")?;
    let wanted: usize = args.get(1).and_then(|n| n.parse().ok()).unwrap_or(400);
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).map_err(|e| format!("{path}: {e}"))?);
    let started = Instant::now();
    let package = Package::open(bytes, &Limits::default()).map_err(|e| e.to_string())?;
    let editor = Editor::open(
        package,
        StoreConfig::default(),
        Arc::new(Monotonic::new()),
        Arc::new(Unlimited),
    )
    .map_err(|e| e.to_string())?;
    let open = ms(started.elapsed());
    let docx = editor.store().snapshot().doc().docx().map_err(|e| e.to_string())?;
    let targets: Vec<String> = docx
        .body()
        .map(|b| {
            b.ids
                .paragraphs()
                .iter()
                .filter(|p| !p.element.children().is_empty())
                .map(|p| p.id)
                .collect()
        })
        .unwrap_or_default();
    let step = (targets.len() / wanted.max(1)).max(1);
    let started = Instant::now();
    let mut made = 0;
    for (k, target) in targets.iter().step_by(step).take(wanted).enumerate() {
        let command = SetText {
            target: target.clone(),
            text: format!("Tracked change number {k} written by a reviewer"),
            policy: RevisionPolicy::Tracked(Tracking {
                author: format!("Writer {}", k % 2),
                date: "2026-11-18T09:00:00Z".to_owned(),
            }),
        };
        if editor.run(&Caller::anonymous(), &command).is_ok_and(|d| d.changed) {
            made += 1;
        }
    }
    let tracking = ms(started.elapsed());
    let doc = editor.store().snapshot().doc().clone();
    let paragraphs = targets.len();
    let mut out = serde_json::Map::new();
    out.insert("document".into(), json!(path));
    out.insert("paragraphs".into(), json!(paragraphs));
    out.insert("open_ms".into(), json!(open));
    out.insert("tracked_edits".into(), json!(made));
    out.insert("tracked_edits_ms".into(), json!(tracking));
    for (name, arguments) in [
        ("list", json!({"doc": "d1", "action": "list"})),
        (
            "list_records",
            json!({"doc": "d1", "action": "list", "detail": "records"}),
        ),
        ("preview_all", json!({"doc": "d1", "action": "preview", "all": true})),
        ("accept_all", json!({"doc": "d1", "action": "accept", "all": true})),
        ("reject_all", json!({"doc": "d1", "action": "reject", "all": true})),
    ] {
        let (time, summary) = best(&doc, &arguments)?;
        out.insert(name.into(), json!({"best_of_3_ms": time, "summary": summary}));
    }
    println!("{}", Value::Object(out));
    Ok(())
}
