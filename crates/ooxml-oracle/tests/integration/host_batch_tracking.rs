//! A host's batch holds its document's tracking mode for its whole run (design phase 3 §4):
//! a `word_set_tracking` from another caller never lands between two of its ops, so every
//! op of a batch is tracked, or none is, as in a serial order -- however often another
//! caller switches the mode beside it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_host::{CallAs, HostSession};
use ooxml_oracle::host_replay::Replayer;
use ooxml_session::{DocId, DocKind};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The ops of each batch, each on a paragraph of its own.
const OPS: usize = 8;

fn opened() -> Arc<HostSession> {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let replayer = Replayer::new(root).unwrap();
    let s = Arc::new(replayer.session(&json!({"session": {}})).unwrap());
    let bytes = std::fs::read(root.join("crates/ooxml-oracle/fixtures/samplelib/sample-long.docx")).unwrap();
    let d = s.open(Arc::from(bytes), DocKind::Docx, "long.docx").unwrap();
    assert_eq!(d, DocId::nth(1));
    s
}

/// The first `wanted` top-level paragraphs a `word_read` names, by id or by place.
fn paragraphs(s: &HostSession, wanted: usize) -> Vec<String> {
    let read = s.call("word_read", json!({"doc": "d1"}), &CallAs::default()).unwrap();
    let text = Value::Object(read.to_json())["data"].as_str().unwrap_or("").to_owned();
    let found: Vec<String> = text
        .lines()
        .filter_map(|line| line.strip_prefix("<!-- ")?.split_whitespace().next())
        .filter(|id| id.starts_with("p:") || id.starts_with("p@body/"))
        .map(str::to_owned)
        .take(wanted)
        .collect();
    assert_eq!(found.len(), wanted, "{text}");
    found
}

/// Whether the text `text` the document now holds was written as a tracked insertion: a
/// `w:ins` opened in its paragraph before it.
fn tracked(document: &str, text: &str) -> bool {
    let at = document
        .find(&format!(">{text}<"))
        .unwrap_or_else(|| panic!("{text} not written"));
    let paragraph = document[..at]
        .rfind("<w:p ")
        .max(document[..at].rfind("<w:p>"))
        .unwrap();
    document[paragraph..at].contains("<w:ins ")
}

#[test]
fn a_tracking_switch_beside_a_batch_never_lands_between_its_ops() {
    let s = opened();
    let targets = paragraphs(&s, OPS);
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        // another caller switching the mode on and off, as fast as it can
        let switcher = scope.spawn(|| {
            let how = CallAs::default();
            let mut switched = 0;
            while !done.load(Ordering::SeqCst) {
                for on in [
                    json!({"doc": "d1", "on": true, "author": "Switcher"}),
                    json!({"doc": "d1", "on": false}),
                ] {
                    let reply = s.call("word_set_tracking", on, &how).unwrap();
                    assert!(!reply.is_error(), "{:?}", reply.to_json());
                    switched += 1;
                }
            }
            switched
        });
        // (the switcher stops whatever the rounds found, so a failure ends the test)
        let mixed = (0..12).find_map(|round| {
            let ops: Vec<Value> = targets
                .iter()
                .enumerate()
                .map(|(k, id)| {
                    json!({"tool": "word_set_text", "arguments": {"doc": "d1", "target": id, "text": format!("b{round}x{k}")}})
                })
                .collect();
            let reply = s.call("batch", json!({"ops": ops}), &CallAs::default()).unwrap();
            if reply.is_error() {
                return Some(format!("round {round}: {:?}", reply.to_json()));
            }
            let head = s.session().format_doc(&DocId::nth(1)).unwrap().stored().head();
            let bytes = head.package().read("word/document.xml").unwrap().unwrap();
            let document = String::from_utf8_lossy(&bytes);
            let modes: Vec<bool> = (0..OPS).map(|k| tracked(&document, &format!("b{round}x{k}"))).collect();
            (!modes.iter().all(|m| *m == modes[0]))
                .then(|| format!("round {round}: the batch's ops written under two modes: {modes:?}"))
        });
        done.store(true, Ordering::SeqCst);
        let switched = switcher.join().unwrap();
        assert_eq!(mixed, None);
        assert!(switched > 0);
    });
}

/// The document's `word/document.xml` at its head.
fn body(s: &HostSession) -> String {
    let head = s.session().format_doc(&DocId::nth(1)).unwrap().stored().head();
    let bytes = head.package().read("word/document.xml").unwrap().unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Phase 3 E2 (the thread groups' test of a step group's revert, on the host): an atomic batch
/// that switches tracking on, writes, and then fails publishes nothing -- the text, the tracking
/// mode and undo as they were -- and one that applies is one step, which one undo takes back.
#[test]
fn an_atomic_batch_that_fails_leaves_the_text_the_tracking_mode_and_undo_as_they_were() {
    let s = opened();
    let targets = paragraphs(&s, 2);
    let how = CallAs::default();
    let set =
        |id: &str, text: &str| json!({"tool": "word_set_text", "arguments": {"doc": "d1", "target": id, "text": text}});
    let first = s
        .call(
            "word_set_text",
            json!({"doc": "d1", "target": targets[0], "text": "kept"}),
            &how,
        )
        .unwrap();
    assert!(!first.is_error(), "{:?}", first.to_json());
    let before = body(&s);
    let refused = s
        .call(
            "batch",
            json!({"atomic": true, "ops": [
                {"tool": "word_set_tracking", "arguments": {"doc": "d1", "on": true, "author": "Bo"}},
                set(&targets[1], "gone"),
                set("p:7FFFFFFF", "nowhere"),
            ]}),
            &how,
        )
        .unwrap();
    assert!(refused.is_error(), "{:?}", refused.to_json());
    assert_eq!(body(&s), before, "nothing of the batch is published");
    let described = Value::Object(s.call("describe", json!({"doc": "d1"}), &how).unwrap().to_json());
    assert_eq!(
        described["data"]["tracking"],
        Value::Null,
        "the tracking mode as it was"
    );
    // undo takes back the edit before the batch: the batch made no step
    let undone = s.call("undo", json!({"doc": "d1"}), &how).unwrap();
    assert!(!undone.is_error(), "{:?}", undone.to_json());
    assert!(!body(&s).contains(">kept<"));
    // a batch that applies is one step, which one undo takes back
    let applied = s
        .call(
            "batch",
            json!({"atomic": true, "ops": [set(&targets[0], "one"), set(&targets[1], "two")]}),
            &how,
        )
        .unwrap();
    assert!(!applied.is_error(), "{:?}", applied.to_json());
    assert!(body(&s).contains(">one<") && body(&s).contains(">two<"));
    let undone = s.call("undo", json!({"doc": "d1"}), &how).unwrap();
    assert!(!undone.is_error(), "{:?}", undone.to_json());
    let now = body(&s);
    assert!(!now.contains(">one<") && !now.contains(">two<"));
}
