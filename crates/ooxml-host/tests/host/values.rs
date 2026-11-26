//! Steps 6 to 8 of a call (ooxml-edit's `test_tools_sources`, `test_tools_datasets` and
//! `test_tools_textfacts`, their generic parts, on the toy format): a declared source's
//! dataset checked, its rows handed to the format; one value of it handed as a value
//! written; a reference in a declared text replaced by its value; an unknown dataset refused
//! by the host with the ones there are.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::toy::{call, session};
use ooxml_session::datasets::DatasetEntry;
use ooxml_tools::datasets::Dataset;
use ooxml_tools::sources::Cell;
use serde_json::json;
use std::sync::Arc;

fn with_sales() -> ooxml_host::HostSession {
    let s = session();
    let row = |region: &str, units: i64| vec![Cell::Text(region.into()), Cell::Int(units.to_string())];
    let sales = Dataset::new(
        vec!["region".into(), "units".into()],
        vec![row("North", 3), row("South", 5)],
    )
    .unwrap()
    .with_keys(1)
    .unwrap();
    s.keep_datasets(vec![DatasetEntry::new("sales", Arc::new(sales), None)])
        .unwrap();
    s
}

#[test]
fn a_declared_sources_rows_are_handed_to_the_format() {
    let s = with_sales();
    let reply = call(&s, "toy_fill", json!({"doc": "d1", "from": "d:sales"}));
    assert_eq!(
        reply["data"]["rows"],
        json!([{"given": "d:sales", "address": "d:sales", "rows": 2}]),
        "{reply}"
    );
    assert_eq!(reply["data"]["written"], json!([]));
    let run = call(&s, "toy_fill", json!({"doc": "d1", "from": "d:sales/#2:#2"}));
    assert_eq!(run["data"]["rows"][0]["rows"], 1, "{run}");
}

#[test]
fn a_source_naming_one_value_is_handed_as_a_value_written_and_where_it_is() {
    let s = with_sales();
    let reply = call(&s, "toy_fill", json!({"doc": "d1", "from": "d:sales/South/units"}));
    assert_eq!(
        reply["data"]["written"],
        json!([{"field": "from", "text": "5", "start": 0, "end": 1}]),
        "{reply}"
    );
    assert_eq!(
        reply["data"]["one"],
        json!([{"field": "from", "shown": "d:sales/South/units", "row": 1, "column": 1}])
    );
    let missing = call(&s, "toy_fill", json!({"doc": "d1", "from": "d:sales/East/units"}));
    assert_eq!(missing["data"]["refused"][0]["field"], "from", "{missing}");
    assert_eq!(missing["data"]["written"], json!([]));
}

#[test]
fn a_reference_in_a_declared_text_is_written_with_its_value() {
    let s = with_sales();
    let reply = call(
        &s,
        "toy_fill",
        json!({"doc": "d1", "text": "North sold {{d:sales/North/units}} units"}),
    );
    assert_eq!(reply["data"]["text"], "North sold 3 units", "{reply}");
    assert_eq!(
        reply["data"]["written"],
        json!([{"field": "text", "text": "3", "start": 11, "end": 12}])
    );
}

#[test]
fn an_unknown_dataset_is_refused_by_the_host_with_the_ones_there_are() {
    let s = with_sales();
    let reply = call(&s, "toy_fill", json!({"doc": "d1", "from": "d:sale"}));
    assert_eq!(reply["error"]["code"], "not_found", "{reply}");
    assert_eq!(reply["error"]["field"], "from");
    let text = call(&s, "toy_fill", json!({"doc": "d1", "text": "{{d:nope/North/units}}"}));
    assert_eq!(text["ok"], false, "{text}");
}

#[test]
fn describe_lists_the_sessions_datasets_never_their_values() {
    let s = with_sales();
    let described = call(&s, "describe", json!({"doc": "d1"}));
    let listed = described["data"]["datasets"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{described}");
    assert!(!listed[0].to_string().contains("North"), "{described}");
    let none = call(&session(), "describe", json!({"doc": "d1"}));
    assert!(none["data"].get("datasets").is_none(), "{none}");
}
