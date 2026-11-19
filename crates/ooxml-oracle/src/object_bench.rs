//! `word-objects-bench <document.docx> [table]`: what the object tools cost on a long
//! document. Each call -- `word_edit_table`'s row inserted, and row and column deletions (with their
//! report of the file's text removed), untracked and tracked -- runs as a whole tool call on a fresh
//! editor of the document, on its middle body table (or `table`); the best of three wall
//! times, as JSON. CPU for the whole run is the caller's to measure (`/usr/bin/time -l`).
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::{Monotonic, Unlimited};
use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ArgType;
use ooxml_tools_word::{word_edit_table, Editing, WordEditTable};
use ooxml_word_edit::{Editor, Tracking};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// The best of three runs of `arguments` on fresh editors of `bytes`: wall ms and summary.
fn best(bytes: &Arc<[u8]>, arguments: &Value, tracking: Option<&Tracking>) -> Result<(f64, String), String> {
    let args = WordEditTable::from_arg(arguments, "").map_err(|e| e.to_string())?;
    let mut times = Vec::new();
    let mut summary = String::new();
    for _ in 0..3 {
        let package = Package::open(Arc::clone(bytes), &Limits::default()).map_err(|e| e.to_string())?;
        let editor = Editor::open(
            package,
            StoreConfig::default(),
            Arc::new(Monotonic::new()),
            Arc::new(Unlimited),
        )
        .map_err(|e| e.to_string())?;
        let opened = editor.store().snapshot().doc().clone();
        opened.docx().map_err(|e| e.to_string())?;
        let caller = Caller::anonymous();
        let editing = Editing {
            editor: &editor,
            caller: &caller,
            opened: &opened,
            tracking,
            doc: "d1",
            cancel: &Never,
            date: "2026-11-17T10:00:00Z",
            layout: None,
        };
        let started = Instant::now();
        let reply = word_edit_table(&args, &editing).to_json();
        times.push((started.elapsed().as_secs_f64() * 1e6).round() / 1e3);
        summary = reply.get("summary").and_then(Value::as_str).unwrap_or("").to_owned();
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
        .ok_or("usage: word-objects-bench <document.docx> [table]")?;
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).map_err(|e| format!("{path}: {e}"))?);
    let package = Package::open(Arc::clone(&bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let editor = Editor::open(
        package,
        StoreConfig::default(),
        Arc::new(Monotonic::new()),
        Arc::new(Unlimited),
    )
    .map_err(|e| e.to_string())?;
    let docx = editor.store().snapshot().doc().docx().map_err(|e| e.to_string())?;
    let tables: Vec<String> = docx
        .body()
        .map(|b| b.ids.tables().iter().map(|t| t.id).collect())
        .unwrap_or_default();
    let table = match args.get(1) {
        Some(table) => table.clone(),
        None => tables
            .get(tables.len() / 2)
            .cloned()
            .ok_or("the document has no table")?,
    };
    let tracking = Tracking {
        author: "Bench".into(),
        date: "2026-11-19T10:00:00Z".into(),
    };
    let mut out = serde_json::Map::new();
    for (action, index) in [("insert_row", 1), ("delete_row", 1), ("delete_column", 0)] {
        let call = json!({"doc": "d1", "table": table, "action": action, "index": index});
        for (name, tracked) in [
            (action.to_owned(), None),
            (format!("{action}, tracked"), Some(&tracking)),
        ] {
            let (ms, summary) = best(&bytes, &call, tracked)?;
            out.insert(name, json!({"ms": ms, "summary": summary}));
        }
    }
    let report = json!({"document": path, "tables": tables.len(), "table": table, "calls": out});
    println!("{report}");
    Ok(())
}
