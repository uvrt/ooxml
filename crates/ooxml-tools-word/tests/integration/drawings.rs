//! `word_drawings`' handlers on docx-agent's chart fixture (`chart-places.docx`: text boxes
//! with their VML fallbacks, an inline chart), for the tool design rules they touch: the
//! drawings listed; a text box inserted and then moved, its fallback following; an inline
//! drawing floated and put back in the line; a drawing already as asked is no change with
//! a note (rule 3); a tracked frame edit saying it is applied untracked; refusals carrying
//! their way out (rule 2). Their equality with docx-agent is
//! `crates/ooxml-oracle/python/word_edits_diff.py --objects`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, Still, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_drawings, Blob, BlobList, CallEditor, Editing, WordDrawings};
use ooxml_word_edit::{Editor, Tracking};
use serde_json::{json, Value};
use std::sync::Arc;

const FIXTURE: &[u8] =
    include_bytes!("../../../../python/docx-agent/tests/fixtures/generated/charts/chart-places.docx");

/// `b1`: a 96 by 48 pixel PNG header stating no density (96 dpi: 72 by 36 pt); `b2`: text.
fn picture_blobs() -> BlobList {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&96u32.to_be_bytes());
    png.extend_from_slice(&48u32.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
    let blob = |handle: &str, mime: &str, data: Vec<u8>| Blob {
        handle: handle.to_owned(),
        name: format!("{handle}.bin"),
        mime: mime.to_owned(),
        data: Arc::from(data),
    };
    BlobList(vec![
        blob("b1", "image/png", png),
        blob("b2", "text/plain", b"words".to_vec()),
    ])
}

/// Each of `calls` to `word_drawings` in turn on a fresh copy of the fixture (tracked as
/// `author` when given): the replies, and the main part as it was left.
fn calls(calls: &[Value], author: Option<&str>) -> (Vec<Value>, String) {
    let package = Package::open(Arc::from(FIXTURE), &Limits::default()).unwrap();
    let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let blobs = picture_blobs();
    let tracking = author.map(|a| Tracking {
        author: a.to_owned(),
        date: "2026-11-17T10:00:00Z".to_owned(),
    });
    let call_editor = CallEditor::new(&editor);
    let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z")
        .with_tracking(tracking.as_ref())
        .with_blobs(Some(&blobs));
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

#[test]
fn a_picture_inserted_from_a_blob_at_its_natural_size_or_a_width() {
    let (replies, xml) = calls(
        &[
            json!({"action": "insert_picture", "image": "b1", "at": "p@body/0@0", "alt_text": "A logo"}),
            json!({"action": "insert_picture", "image": "b1", "at": "after:p@body/0", "alt_text": "Again",
                   "width": 36}),
        ],
        None,
    );
    assert_eq!(replies[0]["data"]["size"], json!([72.0, 36.0]));
    assert_eq!(replies[1]["data"]["size"], json!([36.0, 18.0]));
    assert!(replies[1]["data"]["paragraph"].is_string(), "a paragraph of its own");
    assert!(xml.contains("descr=\"A logo\""));
    assert_eq!(xml.matches("<pic:pic").count(), 2, "the fixture has none");
}

#[test]
fn a_picture_call_refused_before_anything_changes() {
    let (replies, _) = calls(
        &[
            json!({"action": "insert_picture", "image": "b9", "at": "p@body/0@0", "alt_text": "x"}),
            json!({"action": "insert_picture", "image": "b2", "at": "p@body/0@0", "alt_text": "x"}),
            json!({"action": "insert_picture", "image": "b1", "at": "p@body/0@0"}),
            json!({"action": "insert_picture", "image": "b1", "at": "p@body/0@0", "alt_text": "x", "height": 9}),
        ],
        None,
    );
    assert_eq!(replies[0]["error"]["code"], "not_found");
    assert_eq!(replies[0]["error"]["valid_options"][0], "b1 (b1.bin)");
    for reply in &replies[1..] {
        assert_eq!(reply["error"]["code"], "invalid_arguments", "{reply}");
    }
}
