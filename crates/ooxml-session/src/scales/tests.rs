//! ooxml-edit's `test_tools_scales.py`, "reading a scale", ported.

use super::*;
use ooxml_tools::{TryCall, WayOut};
use serde_json::{json, Value};

fn way_out() -> WayOut {
    WayOut::call(TryCall {
        tool: "check".into(),
        arguments: serde_json::Map::new(),
        fill: Vec::new(),
    })
}

fn scale_with(changes: Value) -> Value {
    let mut out = json!({"name": "Score",
        "steps": [{"min": 1, "fill": "C62828", "label": "Low"},
                  {"min": 2.5, "fill": "F9A825"},
                  {"min": 4, "fill": "2E7D32", "label": "High"}],
        "targets": ["a1", "a2"]});
    for (key, value) in changes.as_object().unwrap() {
        out[key] = value.clone();
    }
    out
}

fn en() -> Locale {
    Locale::default()
}

fn nl_be() -> Locale {
    Locale {
        decimal_separator: ',',
        thousands_separator: '.',
        ..Locale::default()
    }
}

fn refused(given: Value) -> String {
    parse(&given, &way_out()).unwrap_err().message().to_owned()
}

#[test]
fn white_or_black_text_is_the_one_of_more_contrast() {
    assert!((ratio("FFFFFF", "000000") - 21.0).abs() < 1e-9);
    assert_eq!(readable_text("2E7D32"), ("FFFFFF", 5.13));
    assert_eq!(readable_text("F9A825"), ("000000", 10.66));
    assert_eq!(readable_text("767676"), ("000000", 4.62));
    let found = parse(&scale_with(json!({})), &way_out()).unwrap();
    let texts: Vec<(&str, &str)> = found.steps.iter().map(|s| (s.text.as_str(), s.rule.as_str())).collect();
    assert_eq!(texts, [("FFFFFF", "auto"), ("000000", "auto"), ("FFFFFF", "auto")]);
    assert!(found.notes.is_empty());
    let forced = parse(
        &scale_with(json!({"steps": [{"min": 1, "fill": "F9A825", "text": "white"}]})),
        &way_out(),
    )
    .unwrap();
    assert_eq!(forced.steps[0].text, "FFFFFF");
    assert!(forced.steps[0].contrast < 4.5);
    assert!(forced.notes[0].contains("below WCAG's 4.5:1") && forced.notes[0].contains("text auto"));
}

#[test]
fn steps_take_ranges_from_their_min_to_the_next() {
    let found = parse(&scale_with(json!({})), &way_out()).unwrap();
    assert_eq!(found.by, By::Range);
    assert_eq!(found.name, "Score");
    let steps: Vec<usize> = ["1", "2.49", "2.5", "3.99", "4", "17"]
        .iter()
        .map(|t| found.due(t, &en()).unwrap().n())
        .collect();
    assert_eq!(steps, [1, 1, 2, 2, 3, 3]);
    assert_eq!(
        found.due("0.5", &en()).unwrap_err(),
        "0.5 is below the first step's min 1"
    );
}

#[test]
fn a_max_makes_a_gap_that_is_meant() {
    let found = parse(
        &scale_with(json!({"steps": [{"min": 1, "max": 2, "fill": "C62828"}, {"min": 3, "fill": "2E7D32"}]})),
        &way_out(),
    )
    .unwrap();
    assert_eq!(
        found.notes,
        ["values above 2 and below 3 are in no step (step 1's max): skipped"]
    );
    assert_eq!(
        found.due("2.5", &en()).unwrap_err(),
        "2.5 is in no step: between step 1's max 2 and step 2's min 3"
    );
}

#[test]
fn steps_out_of_order_or_overlapping_are_refused_with_the_way_out() {
    let message = refused(scale_with(
        json!({"steps": [{"min": 3, "fill": "2E7D32"}, {"min": 1, "fill": "C62828"}]}),
    ));
    assert!(
        message.contains("lowest first") && message.contains("[1.0, 3.0]"),
        "{message}"
    );
    let message = refused(scale_with(
        json!({"steps": [{"min": 1, "max": 3, "fill": "C62828"}, {"min": 2, "fill": "2E7D32"}]}),
    ));
    assert!(message.contains("overlap"));
    let message = refused(scale_with(
        json!({"steps": [{"min": 1, "fill": "C62828"}, {"value": "5", "fill": "2E7D32"}]}),
    ));
    assert!(message.contains("mix min and value"));
    assert!(refused(scale_with(json!({"steps": [{"fill": "C62828"}]}))).contains("says no values"));
    let error = parse(&scale_with(json!({"steps": [{"min": 1, "fill": "green"}]})), &way_out()).unwrap_err();
    assert!(error.message().starts_with("steps[0].fill 'green' is not a colour"));
    assert_eq!(error.field(), Some("scale"));
    assert!(error.to_json()["details"]["example"]["steps"].is_array());
    assert!(parse(&scale_with(json!({"targets": []})), &way_out()).is_err());
    assert!(parse(&scale_with(json!({"name": "Level 2"})), &way_out()).is_err());
}

#[test]
fn exact_values_and_texts() {
    let steps: Vec<Value> = [
        (1, "C62828"),
        (2, "EF6C00"),
        (3, "F9A825"),
        (4, "7CB342"),
        (5, "2E7D32"),
    ]
    .iter()
    .map(|(n, f)| json!({"value": n, "fill": f}))
    .collect();
    let found = parse(&scale_with(json!({ "steps": steps })), &way_out()).unwrap();
    assert_eq!(found.by, By::Value);
    assert_eq!(found.due("3", &en()).unwrap().n(), 3);
    assert_eq!(found.due("3,0", &nl_be()).unwrap().n(), 3);
    assert_eq!(
        found.due("3.5", &en()).unwrap_err(),
        "3.5 is no step's value; steps take 1, 2, 3, 4, 5"
    );
    let message = refused(scale_with(
        json!({"steps": [{"value": 1, "fill": "C62828"}, {"value": 2, "fill": "F9A825"},
        {"value": 4, "fill": "2E7D32"}]}),
    ));
    assert!(message.contains("no step takes 3") && message.contains("min/max"));
    let texts = parse(
        &scale_with(json!({"steps": [{"value": "Low", "fill": "C62828"}, {"value": "High", "fill": "2E7D32"}]})),
        &way_out(),
    )
    .unwrap();
    assert_eq!(texts.by, By::Text);
    assert_eq!(texts.due("  high ", &en()).unwrap().n(), 2);
    assert_eq!(
        texts.due("Medium", &en()).unwrap_err(),
        "'Medium' is no step's text; steps take 'Low', 'High'"
    );
    assert!(parse(
        &scale_with(json!({"steps": [{"value": "Low", "fill": "C62828"}, {"value": "low", "fill": "2E7D32"}]})),
        &way_out()
    )
    .is_err());
}

#[test]
fn a_value_is_one_number_in_the_locale() {
    assert_eq!(read_value("2,5", &nl_be()), Some(2.5));
    assert_eq!(read_value("2.5", &en()), Some(2.5));
    assert_eq!(read_value("1.234,5", &nl_be()), Some(1234.5));
    assert_eq!(read_value("1,234.5", &en()), Some(1234.5));
    assert_eq!(read_value("85 %", &en()), Some(85.0));
    assert_eq!(read_value("3/5", &en()), Some(3.0));
    assert_eq!(read_value("3 out of 5", &en()), Some(3.0));
    assert_eq!(read_value("−2", &en()), Some(-2.0));
    for none in ["n/a", "2 of 3", "Score 3", ""] {
        assert_eq!(read_value(none, &en()), None, "{none}");
    }
}

#[test]
fn show_label_needs_a_label_per_step_none_twice() {
    let labelled = json!([{"min": 1, "fill": "C62828", "label": "Low"},
        {"min": 2.5, "fill": "F9A825", "label": "Mid"},
        {"min": 4, "fill": "2E7D32", "label": "High"}]);
    assert!(
        parse(&scale_with(json!({"steps": labelled, "show": "labels"})), &way_out())
            .unwrap()
            .labels
    );
    assert!(!parse(&scale_with(json!({"show": ""})), &way_out()).unwrap().labels);
    assert!(refused(scale_with(json!({"show": "label"}))).contains("step 2 has none: give every step a label"));
    let twice = json!([{"min": 1, "fill": "C62828", "label": "Low"}, {"min": 2.5, "fill": "F9A825", "label": "low"}]);
    assert!(
        refused(scale_with(json!({"steps": twice, "show": "label"}))).contains("steps 1 and 2 are both labelled 'low'")
    );
    assert!(
        refused(scale_with(json!({"show": "name"}))).contains("value (the text is the value, the default) or label")
    );
}
