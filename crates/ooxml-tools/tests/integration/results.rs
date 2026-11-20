//! The result envelope: today's JSON key for key (the expected texts are what Python's
//! `Result.to_text` writes for the same results), the reply budget with nothing cut
//! silently (rule 4), no-ops as successes with a note (rule 3), and the application-only
//! fields kept out of the model's JSON.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_tools::{Done, Refusal, Timing, ToolError, ToolResult, TryCall, WayOut};
use serde_json::{json, Map, Value};

fn sample() -> ToolResult {
    let mut done = Done::new("Set 2 paragraphs");
    done.changed = vec!["p:A".into(), "p:B".into()];
    done.data = Some(json!({"rows": [1, 2, 3], "é": "ü"}));
    done.refs = vec![("x".into(), "p:A".into())];
    done.total = Some(9);
    let mut result = ToolResult::done(done);
    result.doc = Some("d1".into());
    result.version = Some(3);
    result.warnings.push("top read as space_before".into());
    result
}

fn big() -> ToolResult {
    let mut done = Done::new("s");
    done.changed = (0..40).map(|i| format!("p:{i}")).collect();
    done.data = Some(Value::from("x".repeat(500)));
    let mut result = ToolResult::done(done);
    result.doc = Some("d1".into());
    result.version = Some(1);
    result.warnings.push("w".into());
    result
}

#[test]
fn the_envelope_is_todays() {
    assert_eq!(
        sample().to_text(None),
        r#"{"ok":true,"doc":"d1","version":3,"summary":"Set 2 paragraphs","changed":["p:A","p:B"],"created":[],"removed":[],"renamed":{},"warnings":["top read as space_before"],"checks":{},"data":{"rows":[1,2,3],"é":"ü"},"next_cursor":null,"refs":{"x":"p:A"},"total":9}"#
    );
    let error = ToolError::refusal(Refusal::NotFound, "no paragraph p:Z", WayOut::names(["p:A"]).unwrap()).at("target");
    let mut failed = ToolResult::<Value>::failed(error);
    failed.doc = Some("d1".into());
    failed.version = Some(3);
    assert_eq!(
        failed.to_text(None),
        r#"{"ok":false,"doc":"d1","version":3,"error":{"code":"not_found","message":"no paragraph p:Z","field":"target","valid_options":["p:A"]}}"#
    );
}

#[test]
fn over_budget_the_reply_says_what_did_not_fit_as_today() {
    let expected_600 = r#"{"ok":true,"doc":"d1","version":1,"summary":"s","changed":["p:0","p:1","p:2","p:3","p:4","p:5","p:6","p:7","p:8","p:9","p:10","p:11","p:12","p:13","p:14","p:15","p:16","p:17","p:18","p:19","p:20","p:21","p:22","p:23","p:24","p:25","p:26","p:27","p:28","p:29","p:30","p:31","p:32","p:33","p:34","p:35","p:36","p:37","p:38","p:39"],"created":[],"removed":[],"renamed":{},"warnings":["w"],"checks":{},"data":null,"next_cursor":null,"truncated":"the result was 927 characters, over the 600 limit; ask for less (a range or a page)"}"#;
    assert_eq!(big().to_text(Some(600)), expected_600);
    let expected_280 = r#"{"ok":true,"doc":"d1","version":1,"summary":"s","changed":{"omitted":40},"created":[],"removed":[],"renamed":{},"warnings":{"omitted":1},"checks":{},"data":null,"next_cursor":null,"truncated":"the result was 927 characters, over the 280 limit; ask for less (a range or a page)"}"#;
    assert_eq!(big().to_text(Some(280)), expected_280);
    let expected_250 = r#"{"ok": true, "doc": "d1", "version": 1, "truncated": "the result was 927 characters, over the 250 limit; ask for less (a range or a page)"}"#;
    assert_eq!(big().to_text(Some(250)), expected_250);
    assert_eq!(
        sample().to_text(Some(100)),
        r#"{"ok": true, "doc": "d1", "version": 3, "truncated": "the result was 258 characters, over the 100 limit; ask for less (a range or a page)"}"#
    );
    // The result's own budget, where it is more than the sender's limit.
    let mut wide = big();
    wide.app.budget = Some(5_000);
    assert_eq!(wide.to_text(Some(250)), big().to_text(None));
}

#[test]
fn a_tool_that_says_how_to_ask_gets_the_call_in_the_reply() {
    let mut result = big();
    if let ooxml_tools::Outcome::Done(done) = &mut result.outcome {
        let mut arguments = Map::new();
        arguments.insert("doc".into(), json!("d1"));
        arguments.insert("cursor".into(), json!("c50"));
        done.ask = Some(TryCall {
            tool: "word_read".into(),
            arguments,
            fill: Vec::new(),
        });
    }
    let text = result.to_text(Some(700));
    let reply: Value = serde_json::from_str(&text).unwrap();
    assert!(reply["data"].is_null());
    assert!(reply["truncated"].as_str().unwrap().contains("927 characters"));
    assert_eq!(
        reply["try_call"],
        json!({"tool": "word_read", "arguments": {"doc": "d1", "cursor": "c50"}})
    );
}

#[test]
fn a_no_op_is_a_success_with_a_note() {
    let result = ToolResult::<Value>::nothing_given("added", "items");
    assert!(!result.is_error() && result.app.unchanged);
    assert_eq!(
        result.to_text(None),
        r#"{"ok":true,"doc":null,"version":null,"summary":"Nothing added: items is empty","changed":[],"created":[],"removed":[],"renamed":{},"warnings":["items is empty: nothing was added"],"checks":{},"data":null,"next_cursor":null}"#
    );
    let already = ToolResult::<Value>::no_op("p:A is bold already", "p:A was bold: nothing changed");
    let reply = already.to_json();
    assert_eq!(reply["ok"], json!(true));
    assert_eq!(reply["changed"], json!([]));
    assert_eq!(reply["warnings"], json!(["p:A was bold: nothing changed"]));
}

#[test]
fn application_only_fields_never_reach_the_model() {
    let mut result = sample();
    result.app.touched = vec![json!({"doc": "d1", "block": "p:A"})];
    result.app.timing = Some(Timing {
        total: 0.5,
        lock_wait: 0.1,
        work: 0.4,
        cpu: 0.3,
        held: 0.2,
    });
    result.app.tool = "word_set_text".into();
    result.app.memory = Some(Map::new());
    assert_eq!(result.to_text(None), sample().to_text(None));
}
