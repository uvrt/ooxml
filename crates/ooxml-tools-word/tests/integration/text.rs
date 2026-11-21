//! The text tools' finds within a scope on docx-agent's lists fixture
//! (`tools/make_fixtures.py`): `word_delete`'s find within `within`, or within the target
//! given with it (said in a warning), both refused unless they name the same blocks; a heading's
//! range searched as its span, and named so; `word_insert_text`'s find within `within`. Their
//! equality with docx-agent is the fixtures differential's (`word_edits_diff.py`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, Still, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_delete, word_insert_text, CallEditor, Editing, WordDelete, WordInsertText};
use ooxml_word_edit::Editor;
use serde_json::{json, Value};
use std::sync::Arc;

const FIXTURE: &[u8] =
    include_bytes!("../../../../python/docx-agent/tests/fixtures/generated/lists-and-styles.docx");

/// Each of `calls` (`(tool, arguments)`) in turn on a fresh copy of the fixture: the replies.
fn calls(calls: &[(&str, Value)]) -> Vec<Value> {
    let package = Package::open(Arc::from(FIXTURE), &Limits::default()).unwrap();
    let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let call_editor = CallEditor::new(&editor);
    let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-21T10:00:00Z");
    calls
        .iter()
        .map(|(tool, arguments)| {
            let mut arguments = arguments.clone();
            arguments["doc"] = json!("d1");
            let result = match *tool {
                "word_delete" => word_delete(&WordDelete::accept(arguments, Some("docx")).unwrap().0, &editing),
                _ => word_insert_text(&WordInsertText::accept(arguments, Some("docx")).unwrap().0, &editing),
            };
            Value::Object(result.to_json())
        })
        .collect()
}

#[test]
fn a_find_within_a_block_deletes_the_one_there() {
    let replies = calls(&[
        ("word_delete", json!({"find": "item", "within": "p:5A000004"})),
        ("word_delete", json!({"find": "item", "within": "p:5A000004"})),
    ]);
    assert_eq!(replies[0]["summary"], "Deleted 'item'", "{}", replies[0]);
    assert!(replies[0]["warnings"].as_array().is_none_or(Vec::is_empty), "{}", replies[0]);
    assert_eq!(replies[1]["error"]["code"], "not_found", "{}", replies[1]);
    assert!(
        replies[1]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("occurs nowhere in p:5A000004"),
        "{}",
        replies[1]
    );
}

#[test]
fn a_find_with_a_target_searches_within_it_and_says_so() {
    let replies = calls(&[
        ("word_delete", json!({"find": "item", "target": "p:5A000003"})),
        ("word_delete", json!({"find": "item", "within": "p:5A000003"})),
    ]);
    assert_eq!(replies[0]["summary"], "Deleted 'item'", "{}", replies[0]);
    assert_eq!(
        replies[0]["warnings"],
        json!(["find was searched within target p:5A000003: within names where to search"]),
        "{}",
        replies[0]
    );
    assert_eq!(replies[1]["error"]["code"], "not_found", "{}", replies[1]);
}

#[test]
fn within_and_another_target_are_refused_with_the_way_out() {
    let replies = calls(&[(
        "word_delete",
        json!({"find": "item", "within": "p:5A000003", "target": "p:5A000004"}),
    )]);
    let error = &replies[0]["error"];
    assert_eq!(error["code"], "invalid_arguments", "{error}");
    assert_eq!(error["field"], "target", "{error}");
    assert_eq!(error["valid_options"], json!(["within", "target"]), "{error}");
}

#[test]
fn a_span_holding_several_is_ambiguous() {
    let replies = calls(&[("word_delete", json!({"find": "item", "within": "p:5A000003..p:5A000005"}))]);
    assert_eq!(replies[0]["error"]["code"], "ambiguous", "{}", replies[0]);
    assert_eq!(replies[0]["error"]["valid_options"].as_array().unwrap().len(), 3);
}

#[test]
fn a_heading_range_is_searched_as_its_span() {
    let replies = calls(&[("word_delete", json!({"find": "absent", "within": "h:p:5A000001"}))]);
    let message = replies[0]["error"]["message"].as_str().unwrap();
    assert!(message.contains("occurs nowhere in p:5A000001..p:5A00000E"), "{message}");
}

#[test]
fn an_insertion_beside_a_find_within_a_block() {
    let replies = calls(&[
        ("word_insert_text", json!({"find": "item", "within": "p:5A000004", "text": " two"})),
        ("word_delete", json!({"find": "item two", "within": "p:5A000004"})),
    ]);
    assert_eq!(replies[0]["summary"], "Inserted text at 1 place(s)", "{}", replies[0]);
    assert_eq!(replies[1]["summary"], "Deleted 'item two'", "{}", replies[1]);
}
