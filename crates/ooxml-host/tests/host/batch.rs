//! A batch through the host (design phase 3, section 4): one step however its ops went, its
//! refs and effects kept once, a failed op skipped or an atomic batch refused whole; and a
//! batch made to conflict k times beside a writer, prepared again each time, leaving what one
//! that never conflicted leaves (the conflict-k-times effects test); a dataset made stale
//! between two attempts said stale by the one that published.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::toy::{as_toy, call, package, session, toy};
use ooxml_session::datasets::{DatasetEntry, DocMeta};
use ooxml_session::DocKind;
use ooxml_store::Caller;
use ooxml_tools::datasets::Dataset;
use ooxml_tools::sources::Cell;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;

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

#[test]
fn a_dataset_made_stale_between_two_attempts_is_said_stale_by_the_one_that_published() {
    // a dataset of a second document, read by the batch's first op; as the writer beside
    // the batch makes it run again, that document changes too
    let ops = json!({"ops": [
        {"tool": "toy_fill", "arguments": {"doc": "d1", "from": "d:sales"}},
        {"tool": "toy_set", "arguments": {"doc": "d1", "target": "a", "text": "one"}},
    ]});
    let with_sales = |stale: bool| {
        let s = session();
        let other = s.open(package(), DocKind::Pptx, "other.pptx").unwrap();
        let sales = Dataset::new(vec!["region".into()], vec![vec![Cell::Text("North".into())]]).unwrap();
        let meta = DocMeta {
            doc: other.to_string(),
            kind: DocKind::Pptx,
            name: "other.pptx".into(),
        };
        s.keep_datasets(vec![DatasetEntry::new("sales", Arc::new(sales), Some(&meta))])
            .unwrap();
        let doc = toy(&s);
        as_toy(&*doc).2.store(1, Ordering::SeqCst);
        if stale {
            let second = s.session().format_doc(&other).unwrap();
            let change: Box<dyn FnOnce() + Send> = Box::new(move || {
                let theirs: Arc<[u8]> = b"<t>theirs</t>".as_slice().into();
                crate::toy::put_part(&as_toy(&*second).0, &Caller::named("other"), "toy/b.xml", theirs).unwrap();
            });
            as_toy(&*doc).4.lock().unwrap().push(change);
        }
        let reply = call(&s, "batch", ops.clone());
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(doc.stored().stats().conflicts, 1, "run again once");
        reply
    };
    let current = with_sales(false);
    assert!(!current.to_string().contains("is stale"), "{current}");
    let stale = with_sales(true);
    let said: Vec<&Value> = stale["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w.as_str().unwrap().starts_with("d:sales is stale"))
        .collect();
    assert_eq!(said.len(), 1, "{stale}");
}

#[test]
fn an_application_tool_in_a_batch_or_read_batch_is_refused_with_the_way_out_call_it_alone() {
    let settings = ooxml_host::HostSettings {
        own_tools: vec!["ds_put".into()],
        ..ooxml_host::HostSettings::default()
    };
    let s = crate::toy::session_with(settings);
    let op = json!({"tool": "ds_put", "arguments": {"id": "x"}});
    let ops = json!([op, {"tool": "toy_set", "arguments": {"doc": "d1", "target": "a", "text": "one"}}]);
    let partial = call(&s, "batch", json!({"ops": ops}));
    assert_eq!(partial["ok"], true, "{partial}");
    assert_eq!(partial["data"]["failed"], json!([0]), "the other op runs");
    let refused = &partial["data"]["ops"][0]["error"];
    assert_eq!(
        refused["message"],
        "ds_put is the application's own tool, which cannot run inside a batch: call it alone"
    );
    assert_eq!(refused["field"], "tool");
    let reads = call(&s, "read_batch", json!({"ops": [op]}));
    let refused = &reads["data"]["ops"][0]["error"];
    assert_eq!(
        refused["message"], "ds_put is the application's own tool, which cannot run inside a read_batch: call it alone",
        "{reads}"
    );
    let other = call(&s, "batch", json!({"ops": [{"tool": "no_such", "arguments": {}}]}));
    assert_ne!(
        other["error"]["message"], refused["message"],
        "another unknown tool: as before"
    );
}
