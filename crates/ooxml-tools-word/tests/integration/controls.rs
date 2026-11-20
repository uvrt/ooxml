//! `word_controls`' handlers on docx-agent's controls fixture (`tools/make_fixtures.py`),
//! for the tool design rules they touch: each control listed with what it holds; a check
//! box inserted, then filled; a drop-down filled by an item's value showing its display
//! text; a bound control's custom XML node written with the fill; a control removed
//! keeping its content; a tracked insertion saying the control itself is no revision; a
//! refusal carrying its way out (rule 2). Their equality with docx-agent is
//! `crates/ooxml-oracle/python/word_edits_diff.py --objects`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, Still, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_controls, CallEditor, Editing, WordControls};
use ooxml_word_edit::{Editor, Tracking};
use serde_json::{json, Value};
use std::sync::Arc;

const FIXTURE: &[u8] = include_bytes!("../../../../python/docx-agent/tests/fixtures/generated/controls.docx");

/// Each of `calls` to `word_controls` in turn on a fresh copy of the fixture (tracked as
/// `author` when given): the replies, and the custom XML item the last left.
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
        let (args, _) = WordControls::accept(arguments, Some("docx")).unwrap();
        replies.push(Value::Object(word_controls(&args, &editing).to_json()));
    }
    let head = editor.store().snapshot();
    let item = head.doc().tree("customXml/item1.xml").unwrap().unwrap();
    let text = ooxml_word_model::docx::text::leading_text(item.root().child_elements().next().unwrap());
    (replies, text)
}

fn listed(reply: &Value, id: &str) -> Value {
    reply["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .cloned()
        .unwrap_or(Value::Null)
}

#[test]
fn every_control_is_listed_with_what_it_holds() {
    let (replies, _) = calls(&[json!({"action": "list"})], None);
    let list = &replies[0];
    assert_eq!(listed(list, "cc:303")["value"], "a", "{list}");
    assert_eq!(listed(list, "cc:305")["value"], "2026-01-15", "{list}");
    assert_eq!(listed(list, "cc:306")["value"], true, "{list}");
    assert_eq!(listed(list, "cc:302")["title"], "Notes", "{list}");
    assert_eq!(
        listed(list, "cc:311")["value"],
        "First paragraph inside.\nSecond paragraph inside.",
        "{list}"
    );
}

#[test]
fn a_check_box_inserted_then_filled_is_checked() {
    let (replies, _) = calls(
        &[
            json!({"action": "insert", "find": "After the", "type": "checkbox"}),
            json!({"action": "list"}),
        ],
        None,
    );
    let id = replies[0]["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(listed(&replies[1], &id)["value"], false, "{}", replies[1]);
    let (replies, _) = calls(
        &[
            json!({"action": "insert", "find": "After the", "type": "checkbox"}),
            json!({"action": "fill", "control": id, "value": "yes"}),
            json!({"action": "list"}),
        ],
        None,
    );
    assert_eq!(replies[1]["changed"], json!([id]), "{}", replies[1]);
    assert_eq!(listed(&replies[2], &id)["value"], true, "{}", replies[2]);
}

#[test]
fn a_drop_down_filled_by_a_value_shows_its_display_text() {
    let (replies, _) = calls(
        &[
            json!({"action": "fill", "control": "cc:303", "value": "b"}),
            json!({"action": "list"}),
        ],
        None,
    );
    assert_eq!(listed(&replies[1], "cc:303")["value"], "b", "{}", replies[1]);
}

#[test]
fn a_bound_control_writes_its_node() {
    let (replies, item) = calls(
        &[json!({"action": "fill", "control": "cc:310", "value": "Globex"})],
        None,
    );
    assert!(replies[0].get("error").is_none(), "{}", replies[0]);
    assert_eq!(item, "Globex");
}

#[test]
fn a_control_removed_keeps_its_content() {
    let (replies, _) = calls(
        &[
            json!({"action": "remove", "control": "cc:302"}),
            json!({"action": "list"}),
        ],
        Some("Reviewer"),
    );
    assert_eq!(replies[0]["removed"], json!(["cc:302"]), "{}", replies[0]);
    assert!(listed(&replies[1], "cc:302").is_null());
    assert_eq!(
        replies[0]["warnings"],
        json!(["Word has no revision for a control itself: it is removed untracked"])
    );
}

#[test]
fn a_locked_control_is_refused_with_the_way_out() {
    let (replies, _) = calls(&[json!({"action": "fill", "control": "cc:307", "value": "x"})], None);
    let error = &replies[0]["error"];
    assert_eq!(error["code"], "refused", "{error}");
    assert_eq!(error["try_call"]["arguments"]["action"], "list", "{error}");
}
