//! `word-objects-bench <document.docx> [table]`: what the object tools cost on a long
//! document. Each call -- `word_edit_table`'s rows and columns inserted, copied and deleted (with their
//! report of the file's text removed), cells merged and split; `word_controls`' insert, fill, remove
//! and list; `word_drawings`' text box and shape inserted, a drawing moved, resized and set, and
//! the list -- untracked and tracked -- runs as a whole tool call on a fresh
//! editor of the document, on its middle body table (or `table`); the best of three wall
//! times, as JSON. CPU for the whole run is the caller's to measure (`/usr/bin/time -l`).
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::{Monotonic, Unlimited};
use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ArgType;
use ooxml_tools_word::{
    word_controls, word_drawings, word_edit_table, CallEditor, Editing, WordControls, WordDrawings, WordEditTable,
};
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

/// A tool call made on an editing context: its reply.
type Call<'c> = &'c dyn Fn(&Editing<'_>) -> Value;

/// The best of three runs of `timed` on fresh editors of `bytes`, each after the untimed
/// `setup` calls: wall ms and the timed reply.
fn best_of(
    bytes: &Arc<[u8]>,
    setup: &[Call<'_>],
    timed: Call<'_>,
    tracking: Option<&Tracking>,
) -> Result<(f64, Value), String> {
    let mut times = Vec::new();
    let mut last = Value::Null;
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
        let blobs = picture_blobs();
        let editing = Editing {
            editor: &CallEditor::new(&editor),
            caller: &caller,
            opened: &opened,
            tracking,
            doc: "d1",
            cancel: &Never,
            date: "2026-11-17T10:00:00Z",
            layout: None,
            others: ooxml_tools_word::Others::default(),
            blobs: Some(&blobs),
        };
        for call in setup {
            call(&editing);
        }
        let started = Instant::now();
        last = timed(&editing);
        times.push((started.elapsed().as_secs_f64() * 1e6).round() / 1e3);
    }
    times.sort_by(f64::total_cmp);
    Ok((times.first().copied().unwrap_or(0.0), last))
}

/// The session's blobs a bench call reads: `b1`, a 640 by 480 pixel PNG header stating
/// 144 dpi (the picture inserted; nothing decodes its pixels).
fn picture_blobs() -> ooxml_tools_word::BlobList {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], body: &[u8]| {
        png.extend_from_slice(&u32::try_from(body.len()).unwrap_or(0).to_be_bytes());
        png.extend_from_slice(kind);
        png.extend_from_slice(body);
        png.extend_from_slice(&[0; 4]);
    };
    let header = [&640u32.to_be_bytes()[..], &480u32.to_be_bytes(), &[8, 2, 0, 0, 0]].concat();
    chunk(b"IHDR", &header);
    let density = [&5669u32.to_be_bytes()[..], &5669u32.to_be_bytes(), &[1]].concat();
    chunk(b"pHYs", &density);
    chunk(b"IDAT", &[0; 64]);
    chunk(b"IEND", &[]);
    ooxml_tools_word::BlobList(vec![ooxml_tools_word::Blob {
        handle: "b1".to_owned(),
        name: "bench.png".to_owned(),
        mime: "image/png".to_owned(),
        data: Arc::from(png),
    }])
}

/// The best of three `word_edit_table` calls of `arguments`: wall ms and summary.
fn best(bytes: &Arc<[u8]>, arguments: &Value, tracking: Option<&Tracking>) -> Result<(f64, String), String> {
    let args = WordEditTable::from_arg(arguments, "").map_err(|e| e.to_string())?;
    let (ms, reply) = best_of(
        bytes,
        &[],
        &|editing| Value::Object(word_edit_table(&args, editing).to_json()),
        tracking,
    )?;
    Ok((ms, summary_of(&reply)))
}

fn summary_of(reply: &Value) -> String {
    reply.get("summary").and_then(Value::as_str).unwrap_or("").to_owned()
}

/// `word_controls`' actions on the document's middle body paragraph: a control inserted
/// (text, check box), one filled and one removed after an untimed insertion, the list.
fn controls(
    bytes: &Arc<[u8]>,
    paragraph: &str,
    tracking: &Tracking,
    out: &mut serde_json::Map<String, Value>,
) -> Result<(), String> {
    let call = |arguments: Value| -> Result<WordControls, String> {
        let mut arguments = arguments;
        if let Some(map) = arguments.as_object_mut() {
            map.insert("doc".into(), json!("d1"));
        }
        WordControls::from_arg(&arguments, "").map_err(|e| e.to_string())
    };
    let insert = call(json!({"action": "insert", "at": format!("{paragraph}@0")}))?;
    let checkbox = call(json!({"action": "insert", "at": format!("{paragraph}@0"), "type": "checkbox"}))?;
    let list = call(json!({"action": "list"}))?;
    let inserted = |editing: &Editing<'_>| Value::Object(word_controls(&insert, editing).to_json());
    // the control the untimed insertion makes is the same in every fresh editor
    let (_, made) = best_of(bytes, &[], &inserted, None)?;
    let id = made
        .pointer("/data/id")
        .and_then(Value::as_str)
        .unwrap_or("cc:0")
        .to_owned();
    let fill = call(json!({"action": "fill", "control": id, "value": "Filled in"}))?;
    let remove = call(json!({"action": "remove", "control": id}))?;
    let timed: [(&str, &[Call<'_>], &WordControls); 5] = [
        ("controls insert", &[], &insert),
        ("controls insert checkbox", &[], &checkbox),
        ("controls fill", &[&inserted], &fill),
        ("controls remove", &[&inserted], &remove),
        ("controls list", &[&inserted], &list),
    ];
    for (name, setup, args) in timed {
        for (label, tracked) in [(name.to_owned(), None), (format!("{name}, tracked"), Some(tracking))] {
            let (ms, reply) = best_of(
                bytes,
                setup,
                &|editing| Value::Object(word_controls(args, editing).to_json()),
                tracked,
            )?;
            out.insert(label, json!({"ms": ms, "summary": summary_of(&reply)}));
        }
    }
    Ok(())
}

/// `word_drawings`' actions on the document's middle body paragraph: a picture inserted
/// (in the line; in a paragraph of its own after it, floating), a text box and a shape
/// inserted, the text box moved, resized, set (wrapping and order) after an untimed
/// insertion, the list.
fn drawings(
    bytes: &Arc<[u8]>,
    paragraph: &str,
    tracking: &Tracking,
    out: &mut serde_json::Map<String, Value>,
) -> Result<(), String> {
    let call = |arguments: Value| -> Result<WordDrawings, String> {
        let mut arguments = arguments;
        if let Some(map) = arguments.as_object_mut() {
            map.insert("doc".into(), json!("d1"));
        }
        WordDrawings::from_arg(&arguments, "").map_err(|e| e.to_string())
    };
    let at = format!("{paragraph}@0");
    let text_box = call(json!({"action": "insert_text_box", "at": at, "text": "In the box\nSecond line"}))?;
    let shape = call(json!({"action": "insert_shape", "at": at, "preset": "ellipse", "text": "Round"}))?;
    let picture = call(json!({"action": "insert_picture", "image": "b1", "at": at, "alt_text": "A bench picture"}))?;
    let inline = call(
        json!({"action": "insert_picture", "image": "b1", "at": format!("after:{paragraph}"),
                             "alt_text": "A bench picture", "wrap": "square", "align": "center"}),
    )?;
    let list = call(json!({"action": "list"}))?;
    let inserted = |editing: &Editing<'_>| Value::Object(word_drawings(&text_box, editing).to_json());
    // the drawing the untimed insertion makes is the same in every fresh editor
    let (_, made) = best_of(bytes, &[], &inserted, None)?;
    let id = made
        .pointer("/data/id")
        .and_then(Value::as_str)
        .unwrap_or("d:0")
        .to_owned();
    let moved = call(json!({"action": "move", "target": id, "x": 20, "y": 10}))?;
    let resized = call(json!({"action": "resize", "target": id, "width": 200}))?;
    let set = call(json!({"action": "set", "target": id, "wrap": "tight", "z_order": "front"}))?;
    let timed: [(&str, &[Call<'_>], &WordDrawings); 8] = [
        ("drawings insert_picture", &[], &picture),
        ("drawings insert_picture own paragraph, floating", &[], &inline),
        ("drawings insert_text_box", &[], &text_box),
        ("drawings insert_shape", &[], &shape),
        ("drawings move", &[&inserted], &moved),
        ("drawings resize", &[&inserted], &resized),
        ("drawings set", &[&inserted], &set),
        ("drawings list", &[&inserted], &list),
    ];
    for (name, setup, args) in timed {
        for (label, tracked) in [(name.to_owned(), None), (format!("{name}, tracked"), Some(tracking))] {
            let (ms, reply) = best_of(
                bytes,
                setup,
                &|editing| Value::Object(word_drawings(args, editing).to_json()),
                tracked,
            )?;
            out.insert(label, json!({"ms": ms, "summary": summary_of(&reply)}));
        }
    }
    Ok(())
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
    let calls = [
        ("insert_row", json!({"index": 1})),
        ("delete_row", json!({"index": 1})),
        ("delete_column", json!({"index": 0})),
        ("insert_column", json!({"index": 0})),
        ("copy_row", json!({"index": 1})),
        (
            "merge",
            json!({"first": {"row": 0, "col": 0}, "last": {"row": 1, "col": 1}}),
        ),
        ("split", json!({"first": {"row": 1, "col": 0}, "rows": 2, "columns": 2})),
        (
            "set_cells",
            json!({"cells": [{"row": 0, "col": 0, "text": "Set\nin lines"}, {"row": 1, "col": 1, "text": "x"}]}),
        ),
        (
            "set_cells a column",
            json!({"action": "set_cells", "cells": [{"col": 0, "rows": "*", "text": "Filled"}]}),
        ),
    ];
    for (action, arguments) in calls {
        // the action named, unless the arguments give it (two calls of one action)
        let mut call = json!({"doc": "d1", "table": table, "action": action});
        if let (Some(call), Some(arguments)) = (call.as_object_mut(), arguments.as_object()) {
            call.extend(arguments.clone());
        }
        for (name, tracked) in [
            (action.to_owned(), None),
            (format!("{action}, tracked"), Some(&tracking)),
        ] {
            let (ms, summary) = best(&bytes, &call, tracked)?;
            out.insert(name, json!({"ms": ms, "summary": summary}));
        }
    }
    let paragraphs: Vec<String> = docx
        .body()
        .map(|b| b.ids.paragraphs().iter().map(|p| p.id.clone()).collect())
        .unwrap_or_default();
    if let Some(paragraph) = paragraphs.get(paragraphs.len() / 2) {
        controls(&bytes, paragraph, &tracking, &mut out)?;
        drawings(&bytes, paragraph, &tracking, &mut out)?;
    }
    let report = json!({"document": path, "tables": tables.len(), "table": table, "calls": out});
    println!("{report}");
    Ok(())
}
