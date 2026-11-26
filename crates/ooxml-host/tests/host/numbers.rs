//! `check`'s numbers in the host (ooxml-edit's `test_tools_numbers.py`, its host parts, on the
//! toy format): the fact alone runs no format check; asked beside other facts, the format's
//! check runs with them and the fact is added, its seconds counted where the reply says where
//! its time went; a format that gives no texts is refused with the way out; the index is kept
//! while nothing it was built of changes; a dataset's value typed comes with the call that
//! writes it by reference.
//!
//! Every name and figure is made up.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::toy::{call, empty_session, package_with};
use ooxml_host::{HostSession, HostSettings};
use ooxml_session::datasets::DatasetEntry;
use ooxml_session::DocKind;
use ooxml_tools::datasets::Dataset;
use ooxml_tools::sources::Cell;
use serde_json::json;
use std::sync::Arc;

/// A session with a deck whose texts are `texts` (`toy/text-<at>.xml`), and `parts` beside.
fn deck(texts: &[(&str, &str)], parts: &[(&str, &str)]) -> HostSession {
    let s = empty_session(HostSettings::default());
    let named: Vec<(String, String)> = texts
        .iter()
        .map(|(at, text)| (format!("toy/text-{at}.xml"), format!("<t>{text}</t>")))
        .chain(parts.iter().map(|(n, t)| ((*n).to_owned(), (*t).to_owned())))
        .collect();
    let borrowed: Vec<(&str, &str)> = named.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
    s.open(package_with(&borrowed), DocKind::Pptx, "deck.pptx").unwrap();
    s
}

fn keep_by_region(s: &HostSession) {
    let row = |region: &str, avg: f64, rate: f64| vec![Cell::Text(region.into()), Cell::Float(avg), Cell::Float(rate)];
    let sources = ["North", "South"]
        .iter()
        .map(|r| {
            ["region", "avg", "rate"]
                .iter()
                .map(|c| vec![format!("t:x/{r}/{c}")])
                .collect()
        })
        .collect();
    let dataset = Dataset::new(
        vec!["region".into(), "avg".into(), "rate".into()],
        vec![row("North", 3.138, 0.25), row("South", 1.5, 0.0372)],
    )
    .unwrap()
    .with_sources(sources)
    .unwrap()
    .with_keys(1)
    .unwrap()
    .with_types(vec![None, Some("number".into()), Some("percent".into())])
    .unwrap();
    s.keep_datasets(vec![DatasetEntry::new("by_region", Arc::new(dataset), None)])
        .unwrap();
}

#[test]
fn numbers_alone_run_no_format_check_and_beside_others_are_added() {
    let s = deck(&[("a", "North 3.14 and 25%; 1.5")], &[]);
    keep_by_region(&s);
    let alone = call(&s, "check", json!({"doc": "d1", "include": ["numbers"]}));
    let keys: Vec<&String> = alone["data"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["numbers", "validation"], "{alone}");
    assert_eq!(
        alone["summary"],
        "0 unverified and 3 typed number(s) in 1 text(s), against 4 value(s)"
    );
    let typed = &alone["data"]["numbers"]["texts"][0]["typed"];
    let from: Vec<&str> = typed
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["from"].as_str().unwrap())
        .collect();
    assert_eq!(
        from,
        [
            "d:by_region/North/avg",
            "d:by_region/North/rate",
            "d:by_region/South/avg"
        ]
    );
    assert_eq!(
        typed[0]["fix"],
        json!({"tool": "replace_text", "arguments": {"doc": "d1", "find": "North 3.14 and 25%; 1.5",
            "replace": "North {{d:by_region/North/avg:0.00}} and 25%; 1.5", "expect": "one", "slides": ["a"]}})
    );
    let both = call(&s, "check", json!({"doc": "d1", "include": ["numbers", "fit"]}));
    assert_eq!(both["data"]["include"], json!(["fit"]), "{both}");
    assert!(both["summary"].as_str().unwrap().starts_with("Checked; 0 unverified"));
    // the format's reply says where its time went: the numbers' seconds too, timing last
    let data = both["data"].as_object().unwrap();
    assert_eq!(data.keys().next_back().unwrap(), "timing");
    assert!(data["timing"]["numbers"].as_f64().unwrap() >= 0.0);
    let none = call(&s, "check", json!({"doc": "d1"}));
    assert!(none["data"].get("numbers").is_none());
}

#[test]
fn a_document_that_gives_no_texts_is_refused_with_the_way_out() {
    let s = deck(&[], &[("toy/no-texts.xml", "<t/>")]);
    let refused = call(&s, "check", json!({"doc": "d1", "include": ["numbers"]}));
    assert_eq!(refused["error"]["code"], "invalid_arguments", "{refused}");
    assert!(refused["error"]["message"]
        .as_str()
        .unwrap()
        .contains("leave numbers out of include"));
    assert_eq!(refused["error"]["valid_options"], json!(["validate", "layout"]));
    assert!(call(&s, "check", json!({"doc": "d1"}))["ok"].as_bool().unwrap());
}

#[test]
fn the_index_is_kept_while_nothing_changes() {
    let s = deck(&[("a", "rated 2.66")], &[]);
    call(&s, "check", json!({"doc": "d1", "include": ["numbers"]}));
    let kept = Arc::clone(s.stores().numbers.as_ref().unwrap().index());
    call(&s, "check", json!({"doc": "d1", "include": ["numbers"]}));
    assert!(Arc::ptr_eq(&kept, s.stores().numbers.as_ref().unwrap().index()));
    keep_by_region(&s);
    let found = call(&s, "check", json!({"doc": "d1", "include": ["numbers"]}));
    assert!(!Arc::ptr_eq(&kept, s.stores().numbers.as_ref().unwrap().index()));
    assert_eq!(found["data"]["numbers"]["values"], 4);
}

#[test]
fn a_session_without_values_says_how_to_get_some() {
    let s = deck(&[("a", "rated 2.66")], &[]);
    let data = &call(&s, "check", json!({"doc": "d1", "include": ["numbers"]}))["data"]["numbers"];
    assert_eq!(data["values"], 0);
    assert!(data["notes"][0].as_str().unwrap().contains("define a dataset"));
}
