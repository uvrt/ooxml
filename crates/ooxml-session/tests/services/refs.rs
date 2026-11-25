//! Refs as a session service, on `TestFormat`: ooxml-edit's ref tests ported
//! (`test_tools_ref_owners.py`, the ref tests of `test_tools_aliases.py` and
//! `test_tools_shared.py`), each named after the test it ports.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::test_format::{
    caller, TestFormat, AFTER, MAKE, NAME, REFUSE, T_ADD, T_HALF, T_MAKE, T_NAME, T_REF_NAME, T_SET, T_TITLE,
};
use ooxml_session::{CallRefs, RefMap, RefTable};
use ooxml_tools::TryCall;
use serde_json::{json, Value};

fn refs(pairs: &[(&str, &str)]) -> RefMap {
    pairs.iter().copied().collect()
}

fn opened() -> TestFormat {
    let mut format = TestFormat::default();
    format.open("d1");
    format.open("d2");
    format
}

fn retry() -> TryCall {
    TryCall {
        tool: "t".into(),
        arguments: serde_json::Map::new(),
        fill: Vec::new(),
    }
}

/// `test_reading_refs_at_the_committed_version_does_not_wait_for_an_edit_in_flight`: a reader
/// keeps the refs it began with while a later call's are kept; each state keeps its own.
#[test]
fn refs_read_at_a_version_stay_while_another_call_keeps_its_own() {
    let mut format = opened();
    assert!(format
        .call(None, MAKE, json!({"doc": "d1", "text": "one", "ref": "one"}))
        .ok());
    let first = format.doc("d1").version;
    let read = format.refs["d1"].shared();
    assert!(format
        .call(None, MAKE, json!({"doc": "d1", "text": "two", "ref": "two"}))
        .ok());
    assert_eq!(*read, refs(&[("one", "item:1")]));
    assert_eq!(*format.refs["d1"].at(first).unwrap(), refs(&[("one", "item:1")]));
    assert_eq!(format.table("d1"), refs(&[("one", "item:1"), ("two", "item:2")]));
}

/// `test_a_call_that_only_names_something_is_committed_at_once`.
#[test]
fn a_call_that_only_names_something_keeps_its_ref_at_the_same_version() {
    let mut format = opened();
    assert!(format
        .call(None, MAKE, json!({"doc": "d1", "text": "one", "ref": "one"}))
        .ok());
    let version = format.doc("d1").version;
    let named = format.call(None, NAME, json!({"doc": "d1", "name": "intro", "address": "item:1"}));
    assert!(named.ok() && format.doc("d1").version == version);
    assert_eq!(
        *format.refs["d1"].at(version).unwrap(),
        refs(&[("one", "item:1"), ("intro", "item:1")])
    );
    let used = format.call(None, AFTER, json!({"doc": "d1", "target": "$intro"}));
    assert_eq!(used.summary, "added after item:1");
}

/// `test_pointing_another_callers_ref_elsewhere_is_refused_with_the_way_out`.
#[test]
fn pointing_another_callers_ref_elsewhere_is_refused_with_the_way_out() {
    let mut format = opened();
    let (one, two) = (caller("one"), caller("two"));
    assert!(format
        .call(one.clone(), MAKE, json!({"doc": "d1", "text": "a", "ref": "table"}))
        .ok());
    let version = format.doc("d1").version;
    let refused = format.call(two.clone(), MAKE, json!({"doc": "d1", "text": "b", "ref": "table"}));
    let error = refused.error();
    assert_eq!(
        (error["code"].as_str(), error["field"].as_str()),
        (Some("invalid_arguments"), Some("ref"))
    );
    assert_eq!(error["valid_options"], json!(["table_2"]));
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("another caller") && message.contains("'table_2'"));
    assert!(!error.to_string().contains("\"one\""));
    assert_eq!(error["details"], json!({"ref": "table", "address": "item:1"}));
    assert_eq!(format.doc("d1").version, version);
    assert_eq!(format.doc("d1").content().items, ["a"]);
    // the way out works; using the other caller's ref works
    assert!(format
        .call(two.clone(), MAKE, json!({"doc": "d1", "text": "b", "ref": "table_2"}))
        .ok());
    let used = format.call(two.clone(), AFTER, json!({"doc": "d1", "target": "$table"}));
    assert_eq!(used.summary, "added after item:1");
    // naming the address it has is no change of it; its owner points it elsewhere
    assert!(format
        .call(two, NAME, json!({"doc": "d1", "name": "table", "address": "item:1"}))
        .ok());
    assert_eq!(format.owner("d1", "table").as_deref(), Some("one"));
    assert!(format
        .call(one, NAME, json!({"doc": "d1", "name": "table", "address": "item:3"}))
        .ok());
    assert_eq!(format.table("d1").get("table"), Some("item:3"));
}

/// `test_a_name_another_caller_has_is_refused_before_anything_is_made`.
#[test]
fn a_name_another_caller_has_is_refused_before_anything_is_made() {
    let mut format = opened();
    let (one, two) = (caller("one"), caller("two"));
    assert!(format
        .call(
            one.clone(),
            AFTER,
            json!({"doc": "d1", "target": "item:0", "ref": "spot"})
        )
        .ok());
    for name in ["spot", "Spot"] {
        let refused = format.call(
            two.clone(),
            AFTER,
            json!({"doc": "d1", "target": "item:0", "ref": name}),
        );
        assert_eq!(refused.error()["valid_options"], json!(["spot_2"]), "{name}");
    }
    assert_eq!(format.doc("d1").content().items, ["after item:0"]);
    assert!(format
        .call(one, AFTER, json!({"doc": "d1", "target": "item:0", "ref": "spot"}))
        .ok());
    assert_eq!(format.table("d1"), refs(&[("spot", "item:2")]));
}

/// `test_the_suggested_name_is_free_and_a_ref_name`.
#[test]
fn the_suggested_name_is_free_and_a_ref_name() {
    let mut format = opened();
    let (one, two) = (caller("one"), caller("two"));
    let long = "a".repeat(32);
    for name in ["x", "x_2", &long] {
        assert!(format
            .call(
                one.clone(),
                NAME,
                json!({"doc": "d1", "name": name, "address": "item:1"})
            )
            .ok());
    }
    let refused = format.call(
        two.clone(),
        NAME,
        json!({"doc": "d1", "name": "x", "address": "item:2"}),
    );
    assert_eq!(refused.error()["valid_options"], json!(["x_3"]));
    let refused = format.call(two, NAME, json!({"doc": "d1", "name": long, "address": "item:2"}));
    assert_eq!(
        refused.error()["valid_options"],
        json!([format!("{}_2", "a".repeat(28))])
    );
}

/// `test_a_ref_with_no_context_is_shared_and_a_call_with_none_points_any`.
#[test]
fn a_ref_with_no_caller_is_shared_and_a_call_with_none_points_any() {
    let mut format = opened();
    let (one, two) = (caller("one"), caller("two"));
    let name = |format: &mut TestFormat, by, address: &str| {
        format
            .call(by, NAME, json!({"doc": "d1", "name": "intro", "address": address}))
            .ok()
    };
    assert!(name(&mut format, None, "item:1"));
    assert_eq!(format.owner("d1", "intro"), None);
    assert!(name(&mut format, one, "item:2"));
    assert_eq!(format.owner("d1", "intro").as_deref(), Some("one"));
    assert!(!name(&mut format, two.clone(), "item:3"));
    assert!(name(&mut format, None, "item:4"));
    assert_eq!(format.owner("d1", "intro"), None);
    assert!(name(&mut format, two, "item:5"));
}

/// `test_a_caller_points_its_own_refs_again_within_one_batch`.
#[test]
fn a_caller_points_its_own_refs_again_within_one_batch() {
    let mut format = opened();
    let (one, two) = (caller("one"), caller("two"));
    assert!(format
        .call(two, NAME, json!({"doc": "d1", "name": "other", "address": "item:9"}))
        .ok());
    let (_, ops) = format.batch(
        one,
        &[
            (MAKE, json!({"doc": "d1", "text": "a", "ref": "mine"})),
            (MAKE, json!({"doc": "d1", "text": "b", "ref": "mine"})),
            (NAME, json!({"doc": "d1", "name": "other", "address": "item:1"})),
        ],
    );
    assert!(ops[0].get("error").is_none() && ops[1].get("error").is_none());
    assert_eq!(ops[2]["error"]["valid_options"], json!(["other_2"]));
    assert_eq!(format.table("d1").get("mine"), Some("item:2"));
    assert_eq!(format.table("d1").get("other"), Some("item:9"));
}

/// `test_undo_and_redo_bring_back_each_ref_with_its_owner`.
#[test]
fn undo_and_redo_bring_back_each_ref_with_its_owner() {
    let mut format = opened();
    let (one, two) = (caller("one"), caller("two"));
    assert!(format
        .call(None, MAKE, json!({"doc": "d1", "text": "a", "ref": "x"}))
        .ok());
    assert!(format
        .call(one, MAKE, json!({"doc": "d1", "text": "b", "ref": "x"}))
        .ok());
    assert!(!format
        .call(two.clone(), MAKE, json!({"doc": "d1", "text": "c", "ref": "x"}))
        .ok());
    assert!(format.undo("d1", false));
    assert_eq!(
        (format.table("d1"), format.owner("d1", "x")),
        (refs(&[("x", "item:1")]), None)
    );
    assert!(format.undo("d1", true));
    assert_eq!(format.table("d1"), refs(&[("x", "item:2")]));
    assert_eq!(format.owner("d1", "x").as_deref(), Some("one"));
    assert!(!format
        .call(two, NAME, json!({"doc": "d1", "name": "x", "address": "item:1"}))
        .ok());
}

/// `test_a_ref_name_refusal_says_the_rule_and_a_name_that_would_do`.
#[test]
fn a_ref_name_refusal_says_the_rule_and_a_name_that_would_do() {
    let mut format = opened();
    let error = format
        .call(None, T_MAKE, json!({"doc": "d1", "ref": "cat chart"}))
        .error();
    assert_eq!(error["field"], "ref");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("starts with a lowercase letter") && message.contains("'cat_chart'"));
    assert!(!message.contains('^'));
}

/// `test_a_capitalised_use_of_a_known_ref_says_did_you_mean`.
#[test]
fn a_capitalised_use_of_a_known_ref_says_did_you_mean() {
    let mut format = opened();
    assert!(format.call(None, T_MAKE, json!({"doc": "d1", "ref": "catchart"})).ok());
    let error = format
        .call(None, T_MAKE, json!({"doc": "d1", "target": "$catChart"}))
        .error();
    assert_eq!(error["code"], "not_found");
    assert!(error["message"].as_str().unwrap().contains("did you mean $catchart?"));
    // something that only looks like one stays an address
    let kept = format.call(None, T_MAKE, json!({"doc": "d1", "target": "$A$1"}));
    assert!(kept.ok() && kept.data == json!({"target": "$A$1"}));
}

/// `test_a_camel_case_or_hyphenated_ref_is_kept_folded_with_a_warning`.
#[test]
fn a_camel_case_or_hyphenated_ref_is_kept_folded_with_a_warning() {
    let mut format = opened();
    let made = format.call(None, T_MAKE, json!({"doc": "d1", "ref": "recNow"}));
    assert!(made.ok() && made.refs == refs(&[("rec_now", "item:1")]));
    assert_eq!(made.warnings, ["ref 'recNow' read as 'rec_now': write $rec_now"]);
    assert_eq!(format.table("d1"), refs(&[("rec_now", "item:1")]));
    let made = format.call(None, T_MAKE, json!({"doc": "d1", "ref": "scale-note"}));
    assert!(made.ok() && made.refs == refs(&[("scale_note", "item:1")]));
}

/// `test_a_folded_use_is_read_as_the_ref_with_a_warning`.
#[test]
fn a_folded_use_is_read_as_the_ref_with_a_warning() {
    let mut format = opened();
    assert!(format.call(None, T_MAKE, json!({"doc": "d1", "ref": "rec_now"})).ok());
    let used = format.call(None, T_MAKE, json!({"doc": "d1", "target": "$recNow/p1"}));
    assert!(used.ok() && used.data == json!({"target": "item:1/p1"}));
    assert_eq!(used.warnings, ["target '$recNow/p1' read as '$rec_now/p1'"]);
    // resolved by the handler the same way
    let read = format.call(None, T_NAME, json!({"doc": "d1", "target": "$rec-now"}));
    assert!(read.ok() && read.data == json!({"target": "item:1"}));
    assert_eq!(read.warnings, ["'$rec-now' read as '$rec_now'"]);
    // no ref of that name: an address, as before
    let other = format.call(None, T_MAKE, json!({"doc": "d1", "target": "$otherName"}));
    assert!(other.ok() && other.data == json!({"target": "$otherName"}) && other.warnings.is_empty());
}

/// `test_a_folded_name_that_names_another_address_is_refused`.
#[test]
fn a_folded_name_that_names_another_address_is_refused() {
    let mut format = opened();
    assert!(format
        .call(
            None,
            T_NAME,
            json!({"doc": "d1", "ref": "rec_now", "address": "item:1"})
        )
        .ok());
    let error = format
        .call(None, T_NAME, json!({"doc": "d1", "ref": "recNow", "address": "item:2"}))
        .error();
    assert_eq!(
        (error["code"].as_str(), error["field"].as_str()),
        (Some("invalid_arguments"), Some("ref"))
    );
    assert_eq!(
        error["message"],
        "ref 'recNow' reads as 'rec_now', which already names item:1: give ref 'rec_now' to point it here \
         instead, or a name of its own"
    );
    assert_eq!(format.table("d1"), refs(&[("rec_now", "item:1")]));
    let again = format.call(None, T_NAME, json!({"doc": "d1", "ref": "recNow", "address": "item:1"}));
    assert!(again.ok());
    assert_eq!(again.warnings, ["ref 'recNow' read as 'rec_now': write $rec_now"]);
}

/// `test_a_ref_name_is_asked_before_anything_is_made`.
#[test]
fn a_ref_name_is_asked_before_anything_is_made() {
    let mut format = opened();
    let kept = |format: &mut TestFormat, name: &str| format.call(None, T_REF_NAME, json!({"doc": "d1", "ref": name}));
    assert_eq!(kept(&mut format, "rec_now").data, json!({"kept": "rec_now"}));
    let found = kept(&mut format, "recNow");
    assert!(found.data == json!({"kept": "rec_now"}) && found.warnings.is_empty());
    assert_eq!(kept(&mut format, "big chart").error()["field"], "ref");
    assert!(format
        .call(
            None,
            T_NAME,
            json!({"doc": "d1", "ref": "rec_now", "address": "item:1"})
        )
        .ok());
    assert_eq!(
        kept(&mut format, "recNow").error()["message"],
        "ref 'recNow' reads as 'rec_now', which already names item:1: give ref 'rec_now' to point it here \
         instead, or a name of its own"
    );
}

/// `test_folded_refs_in_a_batch`.
#[test]
fn folded_refs_in_a_batch() {
    let mut format = opened();
    let (reply, ops) = format.batch(
        None,
        &[
            (T_MAKE, json!({"doc": "d1", "ref": "catChart"})),
            (T_MAKE, json!({"doc": "d1", "target": "$catChart"})),
        ],
    );
    assert_eq!(ops[1]["data"], json!({"target": "item:1"}));
    assert_eq!(
        reply.warnings,
        [
            "ops[0]: ref 'catChart' read as 'cat_chart': write $cat_chart",
            "ops[1]: target '$catChart' read as '$cat_chart'"
        ]
    );
    // a use of a ref a failed op would have made says so, folded or not
    let (_, ops) = format.batch(
        None,
        &[
            (T_MAKE, json!({"doc": "d1", "ref": "newOne", "nope": 1})),
            (T_MAKE, json!({"doc": "d1", "target": "$newOne"})),
        ],
    );
    assert!(ops[1]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("$new_one was to be made by ops[0]"));
}

/// `test_a_ref_names_an_object_for_later_calls_and_is_listed`.
#[test]
fn a_ref_names_an_object_for_later_calls_and_is_listed() {
    let mut format = opened();
    let added = format.call(
        None,
        T_ADD,
        json!({"doc": "d1", "items": [{"text": "a", "ref": "first"}, {"text": "b"}]}),
    );
    assert_eq!(added.refs, refs(&[("first", "item:1")]));
    assert!(format
        .call(None, T_SET, json!({"doc": "d1", "target": "$first", "text": "A"}))
        .ok());
    assert_eq!(format.doc("d1").content().items, ["A", "b"]);
    let unknown = format
        .call(None, T_SET, json!({"doc": "d1", "target": "$second", "text": "B"}))
        .error();
    assert_eq!(unknown["code"], "not_found");
    assert_eq!(unknown["valid_options"], json!(["$first"]));
    assert_eq!(unknown["field"], "target");
}

/// `test_a_bad_ref_name_rolls_the_call_back`.
#[test]
fn a_bad_ref_name_rolls_the_call_back() {
    let mut format = opened();
    let result = format.call(
        None,
        T_ADD,
        json!({"doc": "d1", "items": [{"text": "a", "ref": "Bad Name"}]}),
    );
    assert_eq!(result.error()["code"], "invalid_arguments");
    assert!(format.doc("d1").content().items.is_empty() && format.table("d1").is_empty());
}

/// `test_a_batch_runs_its_ops_in_order_as_one_undo_step_with_refs`.
#[test]
fn a_batch_runs_its_ops_as_one_undo_step_with_refs() {
    let mut format = opened();
    let before = format.doc("d1").version;
    let (reply, _) = format.batch(
        None,
        &[
            (T_ADD, json!({"doc": "d1", "items": [{"text": "a", "ref": "x"}]})),
            (T_ADD, json!({"doc": "d1", "items": [{"text": "b", "ref": "y"}]})),
            (T_SET, json!({"doc": "d1", "target": "$x", "text": "A"})),
        ],
    );
    assert_eq!(format.doc("d1").content().items, ["A", "b"]);
    assert_eq!(reply.refs, refs(&[("x", "item:1"), ("y", "item:2")]));
    assert_eq!(format.table("d1"), reply.refs);
    assert!(format.doc("d1").version > before);
    assert!(format.undo("d1", false));
    assert!(format.doc("d1").content().items.is_empty() && format.table("d1").is_empty());
}

/// `test_a_ref_a_failed_op_would_have_made_names_that_op` and
/// `test_an_op_that_fails_half_way_is_rolled_back_alone`.
#[test]
fn a_ref_a_failed_op_would_have_made_names_that_op() {
    let mut format = opened();
    let (reply, ops) = format.batch(
        None,
        &[
            (T_HALF, json!({"doc": "d1", "ref": "chart1"})),
            (T_SET, json!({"doc": "d1", "target": "$chart1", "text": "A"})),
            (T_ADD, json!({"doc": "d1", "items": [{"text": "b"}]})),
        ],
    );
    assert_eq!(
        ops[1]["error"],
        json!({"code": "not_found", "field": "target", "details": {"op": 0}, "valid_options": [],
               "message": "$chart1 was to be made by ops[0], which failed: fix that op and resend both",
               "try_call": {"tool": "t_set", "arguments": {"doc": "d1", "target": "$chart1", "text": "A"}}})
    );
    assert!(reply.refs.is_empty() && format.table("d1").is_empty());
    // a ref in an invalid op counts too; a ref nobody asked for is the usual error
    let (_, ops) = format.batch(
        None,
        &[
            (T_ADD, json!({"doc": "d1", "items": [{"text": 5, "ref": "z"}]})),
            (T_SET, json!({"doc": "d1", "target": "$z", "text": "A"})),
            (T_SET, json!({"doc": "d1", "target": "$nobody", "text": "A"})),
        ],
    );
    assert!(ops[1]["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("$z was to be made by ops[0]"));
    assert_eq!(ops[2]["error"]["message"], "no ref $nobody in d1");
}

/// `test_undo_takes_back_the_refs_of_what_it_undoes`.
#[test]
fn undo_takes_back_the_refs_of_what_it_undoes() {
    let mut format = opened();
    assert!(format
        .call(None, T_ADD, json!({"doc": "d1", "items": [{"text": "a", "ref": "x"}]}))
        .ok());
    format.batch(
        None,
        &[(T_ADD, json!({"doc": "d1", "items": [{"text": "b", "ref": "y"}]}))],
    );
    assert_eq!(format.table("d1"), refs(&[("x", "item:1"), ("y", "item:2")]));
    format.undo("d1", false);
    assert_eq!(format.table("d1"), refs(&[("x", "item:1")]));
    format.undo("d1", false);
    assert!(format.table("d1").is_empty());
    format.undo("d1", true);
    format.undo("d1", true);
    assert_eq!(format.table("d1"), refs(&[("x", "item:1"), ("y", "item:2")]));
    format.undo("d1", false);
    assert!(format
        .call(None, T_ADD, json!({"doc": "d1", "items": [{"text": "c"}]}))
        .ok());
    assert_eq!(format.table("d1"), refs(&[("x", "item:1")]));
    let stale = format.call(None, T_SET, json!({"doc": "d1", "target": "$y", "text": "B"}));
    assert_eq!(stale.error()["code"], "not_found");
}

/// `test_the_application_reads_and_resolves_refs_outside_a_call`.
#[test]
fn the_application_reads_and_resolves_refs_outside_a_call() {
    let mut format = opened();
    let items = json!({"doc": "d1", "items": [{"text": "a", "ref": "first"}, {"text": "b", "ref": "second"}]});
    assert!(format.call(None, T_ADD, items).ok());
    let outside = CallRefs::new(None);
    let table = format.refs.get("d1");
    let resolve = |value: &str| {
        outside
            .resolve_value(value, "d1", table, "value", &retry())
            .map(|(r, _)| r)
    };
    assert_eq!(resolve("$second").unwrap(), "item:2");
    assert_eq!(resolve("$first/r0").unwrap(), "item:1/r0");
    assert_eq!(resolve("item:2").unwrap(), "item:2");
    let missing = resolve("$third").unwrap_err().to_json();
    assert_eq!(missing["code"], "not_found");
    assert_eq!(missing["valid_options"], json!(["$first", "$second"]));
    assert!(missing["message"].as_str().unwrap().contains("no ref $third in d1"));
    assert!(format.table("d2").is_empty());
    format.undo("d1", false);
    assert!(format.table("d1").is_empty());
    // names folded as a call folds them
    assert!(format
        .call(
            None,
            T_ADD,
            json!({"doc": "d1", "items": [{"text": "c", "ref": "rec_now"}]})
        )
        .ok());
    let table = format.refs.get("d1");
    let resolve = |value: &str| {
        outside
            .resolve_value(value, "d1", table, "value", &retry())
            .map(|(r, _)| r)
    };
    assert_eq!(resolve("$recNow").unwrap(), "item:1");
    assert_eq!(resolve("$rec-now/r1").unwrap(), "item:1/r1");
    assert_eq!(resolve("$otherName").unwrap(), "$otherName");
    let misspelt = resolve("$REC_NOW").unwrap_err().to_json();
    assert_eq!(misspelt["code"], "not_found");
    assert!(misspelt["message"].as_str().unwrap().contains("did you mean $rec_now?"));
}

/// `test_a_scoped_undo_takes_back_one_pages_latest_change_only`.
#[test]
fn a_scoped_undo_takes_back_one_pages_latest_change_only() {
    let mut format = opened();
    let title = |page: &str, text: &str, name: Option<&str>| {
        let mut args = json!({"doc": "d2", "page": page, "text": text});
        if let Some(name) = name {
            args["ref"] = Value::from(name);
        }
        args
    };
    assert!(format.call(None, T_TITLE, title("1", "A", Some("one"))).ok());
    assert!(format.call(None, T_TITLE, title("2", "B", Some("two"))).ok());
    assert!(format.call(None, T_TITLE, title("1", "C", None)).ok());
    assert!(format.undo_scope("d2", "2", false));
    let titles = &format.doc("d2").content().titles;
    assert_eq!(
        (titles.get("1").map(String::as_str), titles.get("2")),
        (Some("C"), None)
    );
    assert_eq!(format.table("d2"), refs(&[("one", "page:1")]));
    assert!(!format.undo_scope("d2", "2", false));
    assert!(format.undo_scope("d2", "2", true));
    assert_eq!(
        format.doc("d2").content().titles.get("2").map(String::as_str),
        Some("B")
    );
    assert_eq!(format.table("d2"), refs(&[("one", "page:1"), ("two", "page:2")]));
    // without a scope, undo is the latest change, whoever made it
    assert!(format.undo("d2", false));
    assert_eq!(format.table("d2"), refs(&[("one", "page:1")]));
}

/// Refs survive a close: the table is the host session's, kept beside the document.
#[test]
fn refs_outlive_their_documents_close() {
    let mut format = opened();
    assert!(format
        .call(None, MAKE, json!({"doc": "d1", "text": "a", "ref": "kept"}))
        .ok());
    format.docs.remove("d1");
    assert_eq!(format.table("d1"), refs(&[("kept", "item:1")]));
}

/// A span's ends are resolved each (docx-agent's `reads.refs`), the declared pass leaving
/// the span whole.
#[test]
fn a_spans_ends_are_resolved_each() {
    let mut table = RefTable::new();
    table.keep(&refs(&[("a", "p:A"), ("b", "p:B")]), None, 1);
    let call = CallRefs::new(None);
    let (whole, _) = call
        .resolve_value("$a..$b", "d1", Some(&table), "target", &retry())
        .unwrap();
    assert_eq!(whole, "$a..$b");
    let (span, warnings) = call.resolve_span("$a.. $b", "d1", Some(&table), &retry()).unwrap();
    assert_eq!((span.as_str(), warnings.len()), ("p:A..p:B", 0));
    assert_eq!(
        call.resolve_span("p:X..$b", "d1", Some(&table), &retry()).unwrap().0,
        "p:X..p:B"
    );
    assert_eq!(call.resolve_span("$a", "d1", Some(&table), &retry()).unwrap().0, "p:A");
    let error = call
        .resolve_span("$a..$c", "d1", Some(&table), &retry())
        .unwrap_err()
        .to_json();
    assert_eq!(error["message"], "no ref $c in d1");
}

/// The refusal of a ref the document has not names its refs, or gives the call as the way
/// out when it has none (rule 1).
#[test]
fn a_refusal_with_no_refs_to_offer_gives_the_call() {
    let call = CallRefs::new(None);
    let error = call
        .resolve_value("$x", "d1", None, "target", &retry())
        .unwrap_err()
        .to_json();
    assert_eq!(error["message"], "no ref $x in d1");
    assert_eq!(error["try_call"]["tool"], "t");
    assert_eq!(error["valid_options"], json!([]));
}

/// `refuse` keeps what it is given (`toy_ref_refuse`), the ref resolved first.
#[test]
fn a_refusal_of_a_resolved_target_names_its_address() {
    let mut format = opened();
    assert!(format
        .call(None, MAKE, json!({"doc": "d1", "text": "a", "ref": "made"}))
        .ok());
    let error = format
        .call(None, REFUSE, json!({"doc": "d1", "target": "$made"}))
        .error();
    assert_eq!(error["message"], "item:1 is not for this");
}
