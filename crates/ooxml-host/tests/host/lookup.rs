//! A tool's name read as the model meant it (ooxml-edit's `test_tool_aliases`, `test_hints`).

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::toy::{call, session};
use serde_json::json;

#[test]
fn a_namespace_and_another_spelling_are_read_as_the_tool_with_a_note() {
    let s = session();
    call(&s, "toy_set", json!({"doc": "d1", "target": "a", "text": "x"}));
    let read = call(&s, "functions.toy_read", json!({"doc": "d1", "target": "a"}));
    assert_eq!(read["ok"], true, "{read}");
    assert_eq!(
        read["warnings"],
        json!(["tool 'functions.toy_read' read as 'toy_read'"])
    );
    let notes = call(&s, "toy_note", json!({"doc": "d1", "action": "list"}));
    assert_eq!(notes["warnings"][0], "tool 'toy_note' read as 'toy_notes'");
}

#[test]
fn a_withdrawn_tool_is_refused_with_what_to_call_instead() {
    let s = session();
    let reply = call(&s, "toy_draw", json!({"doc": "d1"}));
    assert_eq!(reply["error"]["code"], "refused");
    assert_eq!(
        reply["error"]["message"],
        "toy_draw is withdrawn until the Rust session has it; write the text with toy_set instead."
    );
    assert_eq!(reply["error"]["details"]["not_supported"], "toy_draw (withdrawn)");
}

#[test]
fn an_unknown_name_names_what_it_most_likely_means() {
    let s = session();
    let hinted = call(&s, "toy_add_comment", json!({"doc": "d1"}));
    assert_eq!(hinted["error"]["code"], "invalid_arguments");
    assert_eq!(
        hinted["error"]["message"],
        "no tool 'toy_add_comment': use toy_notes with action add"
    );
    // the hint's tool first, then the names close to it
    assert_eq!(hinted["error"]["valid_options"][0], "toy_notes");
    let near = call(&s, "toy_reed", json!({"doc": "d1"}));
    assert_eq!(
        near["error"]["message"],
        "no tool 'toy_reed'; the nearest: toy_read, toy_set, toy_notes"
    );
    let nothing = call(&s, "zzz", json!({}));
    let options = nothing["error"]["valid_options"].as_array().unwrap();
    assert!(options.iter().any(|o| o == "toy_read") && options.iter().any(|o| o == "undo"));
    let mut sorted = options.clone();
    sorted.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    assert_eq!(&sorted, options, "every tool, sorted");
}

#[test]
fn an_action_a_tool_lacks_is_read_with_its_verb() {
    let s = session();
    let reply = call(&s, "toy_notes", json!({"doc": "d1", "action": "remove"}));
    assert_eq!(reply["error"]["code"], "invalid_arguments");
    let message = reply["error"]["message"].as_str().unwrap();
    assert!(message.ends_with("; did you mean 'delete'?"), "{message}");
}
