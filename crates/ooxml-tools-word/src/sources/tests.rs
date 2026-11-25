use super::*;
use serde_json::json;

#[test]
fn row_ranges_are_written_as_ooxml_edit_writes_them() {
    assert_eq!(first_rows("t:sales", 9).as_deref(), Some("t:sales/#1:#9"));
    assert_eq!(
        first_rows("d:by_region/#10:", 5).as_deref(),
        Some("d:by_region/#10:#14")
    );
    assert_eq!(first_rows("d2/t:sales", 1).as_deref(), Some("d2/t:sales/#1"));
    assert_eq!(first_rows("d:x/#2:#40", 3).as_deref(), Some("d:x/#2:#4"));
    assert_eq!(first_rows("d:x/a", 3), None);
    assert_eq!(first_rows("d:x", 0), None);
}

#[test]
fn a_value_refused_names_its_argument_and_code() {
    let sources = Sources::from_json(&json!([
        {"given": "d:none", "refused": {"message": "no dataset d:none", "options": []}},
        {"given": "d:sales/#9/Region", "field": "cells[1].from",
         "refused": {"code": "not_found", "message": "d:sales has 3 rows; there is no row #9",
                     "options": ["d:sales/#1/Region", "d:sales/#3/Region"]}},
    ]))
    .unwrap();
    let [source, value] = &sources.refused[..] else {
        panic!("two refusals")
    };
    assert_eq!((source.field.as_deref(), source.code.as_str()), (None, "not_found"));
    assert_eq!(value.field.as_deref(), Some("cells[1].from"));
    let retry = TryCall {
        tool: "word_edit_table".into(),
        arguments: serde_json::Map::new(),
        fill: Vec::new(),
    };
    let error = value_refusal(value, &retry).to_json();
    assert_eq!(error["field"], json!("cells[1].from"));
    assert_eq!(
        error["valid_options"],
        json!(["d:sales/#1/Region", "d:sales/#3/Region"])
    );
    assert!(Sources::from_json(&json!([{"given": "x", "refused": {"message": "m", "code": "nope"}}])).is_none());
}
