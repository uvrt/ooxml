//! A batch through the host (design phase 3, section 4): one step however its ops went, its
//! refs and effects kept once, a failed op skipped or an atomic batch refused whole; and a
//! batch made to conflict k times beside a writer, prepared again each time, leaving what one
//! that never conflicted leaves (the conflict-k-times effects test).

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::toy::{as_toy, call, session, toy};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

fn ops() -> Value {
    json!({"ops": [
        {"tool": "toy_set", "arguments": {"doc": "d1", "target": "a", "text": "one", "ref": "first"}},
        {"tool": "toy_read", "arguments": {"doc": "d1", "target": "$first"}},
        {"tool": "toy_set", "arguments": {"doc": "d1", "target": "b", "text": "two"}},
        {"tool": "toy_read", "arguments": {"doc": "d1", "target": "b"}},
    ]})
}

/// A part's text read, without its XML declaration.
fn text(read: &Value) -> &str {
    let read = read.as_str().unwrap();
    read.rsplit('\n').next().unwrap()
}

/// A batch's reply with what a conflict may change taken out: the version (the writer beside
/// it made versions too).
fn without_version(mut reply: Value) -> Value {
    reply.as_object_mut().unwrap().remove("version");
    reply
}

#[test]
fn a_batch_is_one_step_its_reads_see_its_own_ops_and_its_effects_are_kept_once() {
    let s = session();
    let reply = call(&s, "batch", ops());
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["version"], 1, "one step");
    assert_eq!(reply["refs"], json!({"first": "a"}));
    assert_eq!(
        text(&reply["data"]["ops"][1]["data"]),
        "<t>one</t>",
        "a read in the batch reads its ops"
    );
    assert_eq!(
        text(&reply["data"]["ops"][3]["data"]),
        "<t>two</t>",
        "a read after it, its version"
    );
    assert_eq!(
        reply["summary"],
        "Ran 4 op(s) as one step: Set a; Read a; Set b; Read b"
    );
    assert_eq!(as_toy(&*toy(&s)).3.load(Ordering::SeqCst), 2, "each op's effect once");
    let read = call(&s, "toy_read", json!({"doc": "d1", "target": "$first"}));
    assert_eq!(text(&read["data"]), "<t>one</t>", "its refs kept");
}

#[test]
fn a_failed_op_is_skipped_and_an_atomic_batch_refused_whole() {
    let s = session();
    let ops = json!([
        {"tool": "toy_set", "arguments": {"doc": "d1", "target": "a", "text": "one", "ref": "kept"}},
        {"tool": "toy_read", "arguments": {"doc": "d1", "target": "missing"}},
        {"tool": "toy_set", "arguments": {"doc": "d1", "target": "b", "text": "two"}},
    ]);
    let partial = call(&s, "batch", json!({"ops": ops}));
    assert_eq!(partial["ok"], true, "{partial}");
    assert_eq!(partial["data"]["failed"], json!([1]));
    assert_eq!(partial["data"]["applied"], 2);
    assert_eq!(partial["data"]["ops"][1]["error"]["field"], "arguments.target");
    let atomic = call(&s, "batch", json!({"ops": ops, "atomic": true}));
    assert_eq!(atomic["ok"], false);
    assert_eq!(atomic["error"]["field"], "ops[1].arguments.target");
    assert_eq!(atomic["error"]["details"]["completed_before"], 1);
    assert_eq!(atomic["version"], 1, "nothing applied");
    assert_eq!(
        as_toy(&*toy(&s)).3.load(Ordering::SeqCst),
        2,
        "the refused batch kept nothing"
    );
}

#[test]
fn a_batch_that_conflicts_k_times_leaves_what_one_that_never_did_leaves() {
    let calm = session();
    let alone = call(&calm, "batch", ops());
    for k in 1..=2 {
        let s = session();
        let doc = toy(&s);
        as_toy(&*doc).2.store(k, Ordering::SeqCst);
        let raced = call(&s, "batch", ops());
        assert_eq!(as_toy(&*doc).2.load(Ordering::SeqCst), 0, "the writer wrote {k} times");
        assert_eq!(
            without_version(raced.clone()),
            without_version(alone.clone()),
            "k = {k}"
        );
        assert_eq!(raced["version"], k + 1, "after the writer's {k} steps");
        // the ops' effects, the refs and the step: once, whichever attempt published
        assert_eq!(as_toy(&*doc).3.load(Ordering::SeqCst), 2, "k = {k}");
        assert_eq!(
            doc.stored().numbers().steps.len(),
            k + 1,
            "the writer's steps and one batch step"
        );
        let read = call(&s, "toy_read", json!({"doc": "d1", "target": "$first"}));
        assert_eq!(text(&read["data"]), "<t>one</t>", "the batch's write landed last");
        let attempts = doc.stored().stats();
        assert_eq!(attempts.conflicts, k as u64, "k = {k}");
        assert_eq!(s.log().len(), 2, "one record for the batch");
    }
}
