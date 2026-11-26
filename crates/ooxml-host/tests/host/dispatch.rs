//! One call through the pipeline: the one acceptance and its notes, refs, a retry key's
//! repeat, the checks of a changing call, the call's record, and the session tools' seam
//! (ooxml-edit's `test_dispatch`, `test_refs`, `test_retry_keys`).

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::toy::{call, call_as, session};
use ooxml_host::CallAs;
use serde_json::{json, Map, Value};

#[test]
fn the_one_acceptance_notes_what_it_read_first() {
    let s = session();
    let reply = call(&s, "toy_set", json!({"doc": "d1", "where": "a", "text": "x"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["warnings"], json!(["where read as target"]));
    assert_eq!(reply["version"], 1);
    assert_eq!(reply["doc"], "d1");
}

#[test]
fn a_changing_call_carries_its_formats_checks_and_a_reading_one_none() {
    let s = session();
    let set = call(&s, "toy_set", json!({"doc": "d1", "target": "a", "text": "x"}));
    assert_eq!(set["checks"], json!({"parts": 2}));
    let listed = call(&s, "toy_notes", json!({"doc": "d1", "action": "list"}));
    assert_eq!(listed["checks"], json!({}));
}

#[test]
fn a_ref_defined_is_kept_folded_and_resolved_on_the_declared_paths() {
    let s = session();
    let set = call(
        &s,
        "toy_set",
        json!({"doc": "d1", "target": "a", "text": "x", "ref": "introPart"}),
    );
    assert_eq!(set["refs"], json!({"intro_part": "a"}), "{set}");
    assert!(set["warnings"][0].as_str().unwrap().contains("intro_part"));
    let read = call(&s, "toy_read", json!({"doc": "d1", "target": "$intro_part"}));
    assert!(read["data"].as_str().unwrap().ends_with("<t>x</t>"), "{read}");
    let missing = call(&s, "toy_read", json!({"doc": "d1", "target": "$nothing"}));
    assert_eq!(missing["error"]["code"], "not_found");
    assert_eq!(missing["error"]["valid_options"], json!(["$intro_part"]));
}

#[test]
fn a_retry_key_repeated_makes_nothing_twice() {
    let s = session();
    let first = call(
        &s,
        "toy_set",
        json!({"doc": "d1", "target": "a", "text": "x", "key": "k1"}),
    );
    let again = call(
        &s,
        "toy_set",
        json!({"doc": "d1", "target": "a", "text": "y", "key": "k1"}),
    );
    assert_eq!(first["version"], 1);
    assert_eq!(again["summary"], "already done with key 'k1': nothing new was made");
    assert_eq!(again["changed"], json!(["a"]));
    let read = call(&s, "toy_read", json!({"doc": "d1", "target": "a"}));
    assert!(read["data"].as_str().unwrap().ends_with("<t>x</t>"), "{read}");
}

#[test]
fn a_tool_that_takes_no_key_says_why() {
    let s = session();
    let reply = call(&s, "toy_notes", json!({"doc": "d1", "action": "add", "key": "k"}));
    assert_eq!(
        reply["error"]["message"],
        "toy_notes takes no key: a retry key is for the tools that create something, so a repeat makes \
         nothing twice; this one is safe to repeat as it is -- resend without key"
    );
}

#[test]
fn every_call_is_recorded_by_its_shape_never_its_content() {
    let s = session();
    let mut context = Map::new();
    context.insert("agent".into(), Value::from("b"));
    let how = CallAs {
        roles: None,
        context: Some(context.clone()),
        ..CallAs::default()
    };
    call_as(
        &s,
        "toy_set",
        json!({"doc": "d1", "target": "a", "text": "secret"}),
        &how,
    );
    call(&s, "zzz", json!({}));
    let log = s.log();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].tool, "toy_set");
    assert_eq!(log[0].docs, vec!["d1".to_owned()]);
    assert_eq!(log[0].shape["text"], "str[6]");
    assert_eq!(log[0].arguments, None);
    assert_eq!(log[0].context, Some(context));
    assert!(!log[1].ok);
    assert_eq!(log[1].code.as_deref(), Some("invalid_arguments"));
}

#[test]
fn a_calls_timing_gives_its_threads_cpu_where_the_clock_counts_it() {
    // the toy clock counts 2 ms of CPU at every reading: one at the call's start, one at its end
    let s = session();
    let result = s
        .call(
            "toy_set",
            json!({"doc": "d1", "target": "a", "text": "x"}),
            &CallAs::default(),
        )
        .unwrap();
    let timing = result.app.timing.unwrap();
    assert_eq!(timing.cpu, Some(0.002));
}
