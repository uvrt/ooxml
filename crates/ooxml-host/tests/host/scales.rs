//! `check`'s scale in the host (ooxml-edit's `test_tools_scales.py`, its host parts, on the
//! toy format): the fact alone runs no format check; beside other facts it is added to the
//! format's check and to numbers; a scale that is none is refused with an example.
//!
//! Every name and figure is made up.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::toy::{call, empty_session, package_with};
use ooxml_host::{HostSession, HostSettings};
use ooxml_session::DocKind;
use serde_json::{json, Value};

fn deck() -> HostSession {
    let s = empty_session(HostSettings::default());
    let parts = [
        ("toy/text-a.xml", "<t>1,5</t>"),
        ("toy/text-b.xml", "<t>4.2</t>"),
        ("toy/text-c.xml", "<t>n/a</t>"),
    ];
    s.open(package_with(&parts), DocKind::Pptx, "deck.pptx").unwrap();
    s
}

fn scale(targets: &[&str]) -> Value {
    json!({"name": "Score", "steps": [{"min": 1, "fill": "C62828", "label": "Low"},
        {"min": 4, "fill": "2E7D32", "label": "High"}], "targets": targets})
}

#[test]
fn a_scale_alone_runs_no_format_check_and_says_each_value_s_step() {
    let s = deck();
    let alone = call(&s, "check", json!({"doc": "d1", "scale": scale(&["a", "b", "c", "d"])}));
    let keys: Vec<&String> = alone["data"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["scale", "validation"], "{alone}");
    let fact = &alone["data"]["scale"];
    assert_eq!(fact["checked"], 2);
    assert_eq!(
        fact["none"],
        json!([{"target": "a", "value": "1,5", "due": 1}, {"target": "b", "value": "4.2", "due": 2}])
    );
    assert_eq!(
        fact["skipped"][0],
        json!({"target": "c", "value": "n/a", "why": "'n/a' is not one number"})
    );
    assert_eq!(fact["skipped"][1], json!({"target": "d", "why": "no text d"}));
    assert_eq!(fact["fix"]["tool"], "style_by_value");
    assert_eq!(fact["fix"]["arguments"]["scale"]["targets"], json!(["a", "b"]));
    assert_eq!(
        alone["summary"],
        "scale: 0 of 2 value(s) in their step, 2 not, 2 skipped"
    );
}

#[test]
fn a_scale_beside_other_facts_is_added_after_them() {
    let s = deck();
    let both = call(
        &s,
        "check",
        json!({"doc": "d1", "include": ["fit"], "scale": scale(&["a"])}),
    );
    assert_eq!(both["data"]["include"], json!(["fit"]), "{both}");
    assert!(both["data"]["scale"]["none"].is_array());
    assert!(both["summary"].as_str().unwrap().starts_with("Checked; scale: 0 of 1"));
    let numbers = call(
        &s,
        "check",
        json!({"doc": "d1", "include": ["numbers"], "scale": scale(&["a"])}),
    );
    let keys: Vec<&String> = numbers["data"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["numbers", "scale", "validation"], "{numbers}");
}

#[test]
fn a_scale_that_is_none_is_refused_with_an_example() {
    let s = deck();
    let given = json!({"doc": "d1", "scale": {"steps": [{"min": 3, "fill": "2E7D32"}, {"min": 1, "fill": "C62828"}],
        "targets": ["a"]}});
    let refused = call(&s, "check", given);
    assert_eq!(refused["error"]["code"], "invalid_arguments", "{refused}");
    assert!(refused["error"]["message"].as_str().unwrap().contains("[1.0, 3.0]"));
    assert_eq!(refused["error"]["field"], "scale");
}
