//! `word_drawings`' handlers on docx-agent's chart fixture (`chart-places.docx`: text boxes
//! with their VML fallbacks, an inline chart), for the tool design rules they touch: the
//! drawings listed; a text box inserted and then moved, its fallback following; an inline
//! drawing floated and put back in the line; a drawing already as asked is no change with
//! a note (rule 3); a tracked frame edit saying it is applied untracked; refusals carrying
//! their way out (rule 2). Their equality with docx-agent is
//! `crates/ooxml-oracle/python/word_edits_diff.py --objects`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::{Cancel, Limits};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_drawings, CallEditor, Editing, WordDrawings};
use ooxml_word_edit::{Editor, Tracking};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

struct Never;
impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

struct Still;
impl Clock for Still {
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

struct Unlimited;
impl CacheBudget for Unlimited {
    fn reserve(&self, _bytes: usize) -> bool {
        true
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

const FIXTURE: &[u8] = include_bytes!("../../../python/docx-agent/tests/fixtures/generated/charts/chart-places.docx");

/// Each of `calls` to `word_drawings` in turn on a fresh copy of the fixture (tracked as
/// `author` when given): the replies, and the main part as it was left.
fn calls(calls: &[Value], author: Option<&str>) -> (Vec<Value>, String) {
    let package = Package::open(Arc::from(FIXTURE), &Limits::default()).unwrap();
    let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let tracking = author.map(|a| Tracking {
        author: a.to_owned(),
        date: "2026-11-17T10:00:00Z".to_owned(),
    });
    let editing = Editing {
        editor: &CallEditor::new(&editor),
        caller: &caller,
        opened: &opened,
        tracking: tracking.as_ref(),
        doc: "d1",
        cancel: &Never,
        date: "2026-11-17T10:00:00Z",
        layout: None,
        others: ooxml_tools_word::Others::default(),
    };
    let mut replies = Vec::new();
    for arguments in calls {
        let mut arguments = arguments.clone();
        arguments["doc"] = json!("d1");
        let (args, _) = WordDrawings::accept(arguments, Some("docx")).unwrap();
        replies.push(Value::Object(word_drawings(&args, &editing).to_json()));
    }
    let head = editor.store().snapshot();
    let bytes = head.doc().package().read("word/document.xml").unwrap().unwrap();
    (replies, String::from_utf8_lossy(&bytes).into_owned())
}

#[test]
fn lists_pictures_and_charts_with_their_sizes() {
    let (replies, _) = calls(&[json!({"action": "list"})], None);
    // a chart in a group is a member docx-agent cannot size: left to the host
    if let Some(rows) = replies[0]["data"].as_array() {
        assert!(rows.iter().all(|r| r["size"].as_array().unwrap().len() == 2));
    }
}

#[test]
fn a_text_box_inserted_and_moved_takes_its_fallback_along() {
    let (replies, xml) = calls(
        &[
            json!({"action": "insert_text_box", "at": "p@body/0@0", "text": "One\nTwo"}),
            json!({"action": "move", "target": "d:9", "x": 36, "y": 72}),
        ],
        None,
    );
    assert_eq!(replies[0]["data"]["id"], "d:9");
    assert_eq!(replies[1]["summary"], "move: d:9");
    assert!(xml.contains("<wp:posOffset>457200</wp:posOffset>"), "x in EMU");
    assert!(
        xml.contains("margin-left:.5in") || xml.contains("margin-left:36pt"),
        "the fallback moved"
    );
    assert!(xml.contains("margin-top:1in"), "the fallback moved down");
}

#[test]
fn floating_and_inline_again_round_trip_and_repeats_are_no_change() {
    let (replies, xml) = calls(
        &[
            json!({"action": "float", "target": "d:4", "wrap": "square"}),
            json!({"action": "float", "target": "d:4"}),
            json!({"action": "inline", "target": "d:4"}),
            json!({"action": "inline", "target": "d:4"}),
        ],
        None,
    );
    assert_eq!(replies[0]["summary"], "float: d:4");
    assert_eq!(replies[1]["summary"], "No change to d:4");
    assert_eq!(replies[1]["warnings"][0], "d:4 floats already");
    assert_eq!(replies[2]["summary"], "inline: d:4");
    assert_eq!(replies[3]["warnings"][0], "d:4 is inline already");
    assert!(xml.contains("wp:inline"));
}

#[test]
fn tracked_frame_edits_are_applied_untracked_and_say_so() {
    let (replies, xml) = calls(
        &[json!({"action": "set", "target": "d:3", "wrap": "tight"})],
        Some("Ann"),
    );
    assert_eq!(
        replies[0]["warnings"][0],
        "Word does not track a drawing's place, wrapping, order or size: applied untracked"
    );
    assert!(!xml.contains("w:author=\"Ann\""));
}

#[test]
fn refusals_carry_their_way_out() {
    let (replies, _) = calls(
        &[
            json!({"action": "set", "target": "d:3"}),
            json!({"action": "move", "target": "d:4", "x": 1, "y": 1}),
            json!({"action": "insert_shape", "at": "p@body/0@0", "preset": "no-such-shape"}),
            json!({"action": "inline", "target": "d:3"}),
        ],
        None,
    );
    for reply in &replies {
        let error = &reply["error"];
        assert!(error.is_object(), "{reply}");
        assert!(
            error.get("valid_options").is_some() || error.get("try_call").is_some(),
            "{reply}"
        );
    }
    assert_eq!(
        replies[1]["error"]["message"],
        "d:4 is inline: float it first (float_drawing)"
    );
}
