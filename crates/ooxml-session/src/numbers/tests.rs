//! ooxml-edit's `test_tools_numbers.py`, "matching" and the fact, ported (the toys' parts
//! that are no host's).

use super::*;
use ooxml_tools::datasets::Dataset;
use ooxml_tools::sources::Cell;
use std::sync::Arc;

fn cell(value: &serde_json::Value) -> Cell {
    Cell::from_json(value).unwrap()
}

fn dataset(rows: &[serde_json::Value], types: Option<&[Option<&str>]>, formats: Option<&[Option<&str>]>) -> Dataset {
    let width = rows[0].as_array().unwrap().len();
    let columns = (0..width).map(|i| format!("c{i}")).collect();
    let rows = rows
        .iter()
        .map(|r| r.as_array().unwrap().iter().map(cell).collect())
        .collect();
    let mut found = Dataset::new(columns, rows).unwrap();
    if let Some(types) = types {
        found = found
            .with_types(types.iter().map(|t| t.map(str::to_owned)).collect())
            .unwrap();
    }
    if let Some(formats) = formats {
        found = found
            .with_formats(formats.iter().map(|t| t.map(str::to_owned)).collect())
            .unwrap();
    }
    found
}

fn index(rows: &[serde_json::Value], types: Option<&[Option<&str>]>, formats: Option<&[Option<&str>]>) -> Index {
    Index::new(vec![Source {
        address: "d:x".into(),
        shown: "d:x".into(),
        dataset: Arc::new(dataset(rows, types, formats)),
    }])
}

fn nl_be() -> Locale {
    Locale {
        decimal_separator: ',',
        thousands_separator: '.',
        ..Locale::default()
    }
}

fn all() -> NumberRules {
    NumberRules {
        whole_numbers: true,
        ..NumberRules::default()
    }
}

fn matches_in(index: &Index, text: &str, locale: &Locale) -> Vec<Vec<f64>> {
    let (found, _) = find_numbers(text, locale, &all(), &[]).unwrap();
    found
        .iter()
        .map(|number| {
            let mut values: Vec<f64> = index
                .matches(number)
                .iter()
                .map(|(i, _)| index.values[*i].number)
                .collect();
            values.sort_by(f64::total_cmp);
            values
        })
        .collect()
}

fn en() -> Locale {
    Locale::default()
}

#[test]
fn a_number_matches_every_value_that_rounds_to_it() {
    let index = index(
        &[
            json!(["a", 2.66]),
            json!(["b", 2.75]),
            json!(["c", 2.751]),
            json!(["d", 2.64]),
        ],
        None,
        None,
    );
    assert_eq!(matches_in(&index, "2,7", &nl_be()), vec![vec![2.66, 2.75]]);
    assert_eq!(matches_in(&index, "2.66 and 2.6", &en()), vec![vec![2.66], vec![2.64]]);
}

#[test]
fn a_value_shown_rounded_matches_as_shown() {
    let index = index(&[json!(["a", 2.649])], None, Some(&[None, Some("0.00")]));
    assert_eq!(matches_in(&index, "2.7", &en()), vec![vec![2.649]]);
    assert_eq!(matches_in(&index, "2.6", &en()), vec![vec![2.649]]);
}

#[test]
fn percentages_match_fractions_and_points() {
    let index = index(
        &[json!(["a", 0.853, 85.3]), json!(["b", 0.12, 3])],
        Some(&[None, Some("percent"), None]),
        None,
    );
    assert_eq!(matches_in(&index, "85%", &en()), vec![vec![0.853, 85.3]]);
    assert_eq!(matches_in(&index, "85.3 %", &en()), vec![vec![0.853, 85.3]]);
    assert_eq!(matches_in(&index, "12.0 %", &en()), vec![vec![0.12]]);
}

#[test]
fn a_percentage_matches_only_when_written_as_one() {
    let index = index(
        &[
            json!(["a", 0.27, 0.27]),
            json!(["b", 0.31, 0.31]),
            json!(["c", 0.3, 30.0]),
        ],
        Some(&[None, Some("percent"), None]),
        None,
    );
    assert_eq!(matches_in(&index, "30 %", &en()), vec![vec![0.3, 30.0]]);
    assert_eq!(matches_in(&index, "27 procent", &nl_be()), vec![vec![0.27, 0.27]]);
    assert_eq!(matches_in(&index, "300‰", &en()), vec![vec![0.3]]);
    assert_eq!(matches_in(&index, "0,3 gemiddeld", &nl_be()), vec![vec![0.27, 0.31]]);
    assert_eq!(
        matches_in(&index, "0.30 and 30.0 and 27.0", &en()),
        vec![vec![], vec![30.0], vec![]]
    );
}

#[test]
fn a_percent_format_marks_a_percentage_too() {
    let index = index(&[json!(["a", 0.31, 0.31])], None, Some(&[None, Some("0%"), None]));
    assert_eq!(
        matches_in(&index, "0.31 and 31%", &en()),
        vec![vec![0.31], vec![0.31, 0.31]]
    );
}

#[test]
fn a_bare_number_is_unverified_against_a_percentage_and_its_nearest_are_plain() {
    let index = index(&[json!(["a", 0.27, 2.5])], Some(&[None, Some("percent"), None]), None);
    let (found, _) = find_numbers("0.3", &en(), &all(), &[]).unwrap();
    assert!(index.matches(&found[0]).is_empty());
    let near: Vec<f64> = index
        .nearest(&found[0], 2)
        .iter()
        .map(|(i, _, _)| index.values[*i].number)
        .collect();
    assert_eq!(near, [2.5]);
}

#[test]
fn a_written_sign_must_agree() {
    let index = index(&[json!(["a", -1.25]), json!(["b", 1.3])], None, None);
    assert_eq!(
        matches_in(&index, "-1.25 and 1.25 and -1.30", &en()),
        vec![vec![-1.25], vec![-1.25], vec![]]
    );
}

#[test]
fn a_scaled_number_matches_the_value_as_it_is_and_scaled() {
    let index = index(&[json!(["a", 2_500_000.0]), json!(["b", 2.5])], None, None);
    assert_eq!(matches_in(&index, "2.5m", &en()), vec![vec![2.5, 2_500_000.0]]);
}

/// A report's surroundings: a locale, the document `d1`, and references written as `facts`
/// gives them.
struct Env {
    locale: Locale,
    facts: Vec<(&'static str, &'static str)>,
}

impl NumbersEnv for Env {
    fn locale(&self) -> &Locale {
        &self.locale
    }
    fn doc(&self) -> Option<&str> {
        Some("d1")
    }
    fn fact(&self, address: &str) -> Option<String> {
        self.facts
            .iter()
            .find(|(a, _)| *a == address)
            .map(|(_, t)| (*t).to_owned())
    }
}

/// `test_tools_numbers.py`'s workbook table `t:Scores`, as its resolver reads it.
fn scores() -> Index {
    let rows = vec![
        vec![
            cell(&json!("door")),
            cell(&json!(2.66)),
            cell(&json!(0.853)),
            cell(&json!(41)),
        ],
        vec![
            cell(&json!("window")),
            cell(&json!(2.549)),
            cell(&json!(0.12)),
            cell(&json!(17)),
        ],
        vec![
            cell(&json!("roof")),
            cell(&json!(-1.25)),
            cell(&json!(0.5)),
            cell(&json!(9)),
        ],
    ];
    let columns = ["topic", "mean", "share", "count"].map(str::to_owned).to_vec();
    let sources = (0..3)
        .map(|r| {
            (0..4)
                .map(|c| vec![format!("Scores!{}{}", char::from(b'A' + c), r + 2)])
                .collect()
        })
        .collect();
    let dataset = Dataset::new(columns, rows)
        .unwrap()
        .with_sources(sources)
        .unwrap()
        .with_keys(1)
        .unwrap()
        .with_formats(vec![None, Some("0.00".into()), Some("0.0%".into()), Some("0".into())])
        .unwrap();
    Index::new(vec![Source {
        address: "d2/t:Scores".into(),
        shown: "t:Scores".into(),
        dataset: Arc::new(dataset),
    }])
}

fn text(at: &str, said: &str, slide: &str) -> NumberText {
    let mut scope = Map::new();
    scope.insert("slides".into(), json!([slide]));
    let mut place = Map::new();
    place.insert("slide".into(), json!(slide));
    NumberText {
        at: at.into(),
        text: said.into(),
        written: Vec::new(),
        scope,
        place,
    }
}

#[test]
fn numbers_are_listed_unverified_with_the_nearest_values_and_typed_with_their_fix() {
    let env = Env {
        locale: nl_be(),
        facts: vec![
            ("d2/t:Scores/door/mean:0.0", "2,7"),
            ("d2/t:Scores/window/mean:0.0", "2,5"),
        ],
    };
    let items = [text("256.2", "Doors rated 2,7/5, windows 2,6 and roofs 2,3.", "s:256")];
    let data = report(
        &env,
        &scores(),
        Vec::new(),
        &items,
        &NumberRules::default(),
        &["slides"],
    )
    .unwrap();
    let first = &data["texts"][0];
    assert_eq!(first["at"], "256.2");
    assert_eq!(first["slide"], "s:256");
    assert_eq!(
        first["unverified"],
        json!([{"text": "2,3", "near": [
            {"value": "2,55", "from": "t:Scores/window/mean", "sources": ["Scores!B3"]},
            {"value": "2,66", "from": "t:Scores/door/mean", "sources": ["Scores!B2"]}]}])
    );
    assert_eq!(
        first["typed"][0]["fix"],
        json!({"tool": "replace_text", "arguments": {"doc": "d1", "find": "Doors rated 2,7/5, windows 2,6",
            "replace": "Doors rated {{t:Scores/door/mean:0.0}}/5, windows 2,6", "expect": "one",
            "slides": ["s:256"]}})
    );
    // a value rounded twice (2.549 shown as 2,55, written 2,6): no fix
    assert!(first["typed"][1].get("fix").is_none());
    assert_eq!(data["left_aside"], json!({"whole": 1}));
    assert_eq!(data["values"], 9);
    assert_eq!(data["note"], NOTE);
    assert_eq!(
        summary(&data),
        "1 unverified and 2 typed number(s) in 1 text(s), against 9 value(s)"
    );
}

#[test]
fn the_lists_are_capped_and_what_did_not_fit_is_counted() {
    let env = Env {
        locale: en(),
        facts: Vec::new(),
    };
    let items: Vec<NumberText> = (2..6)
        .map(|n| text(&format!("256.{n}"), "7.1 7.2 7.3 7.4 7.5 7.6 7.7", "s:256"))
        .collect();
    let rules = NumberRules {
        texts: 2,
        per_text: 3,
        ..NumberRules::default()
    };
    let data = report(&env, &scores(), Vec::new(), &items, &rules, &["slides", "pages"]).unwrap();
    assert_eq!(data["texts"].as_array().unwrap().len(), 2);
    assert_eq!(data["texts_left_out"], 2);
    assert_eq!(data["texts"][0]["unverified"].as_array().unwrap().len(), 3);
    assert_eq!(data["texts"][0]["more"], 4);
    assert_eq!(data["unverified"], 28);
    assert_eq!(
        data["left_out_hint"],
        "check fewer at a time to see the rest (slides, pages)"
    );
}

#[test]
fn a_session_without_values_says_how_to_get_some() {
    let env = Env {
        locale: en(),
        facts: Vec::new(),
    };
    let data = report(
        &env,
        &Index::new(Vec::new()),
        Vec::new(),
        &[text("256.2", "rated 2.66", "s:256")],
        &NumberRules::default(),
        &[],
    )
    .unwrap();
    assert_eq!(data["values"], 0);
    assert_eq!(data["texts"], json!([]));
    assert!(data["notes"][0].as_str().unwrap().contains("define a dataset"));
}

use serde_json::json;
