//! The dataset as a value: ooxml-edit's `test_tools_datasets.py` and `test_tools_sources.py`
//! value tests ported, each named after the one it ports. Every figure is made up.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::type_complexity
)]

use super::{dataset_id, row_range, rows_text, Cache, Dataset, Labels, LazyMarkdown, Markdown, Records, TYPE_LABELS};
use crate::sources::{Cell, Day};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

fn text(t: &str) -> Cell {
    Cell::Text(t.into())
}

fn int(n: i64) -> Cell {
    Cell::Int(n.to_string())
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

fn sources(rows: &[&[&[&str]]]) -> Vec<Vec<Vec<String>>> {
    rows.iter()
        .map(|row| row.iter().map(|cell| names(cell)).collect())
        .collect()
}

/// `test_a_dataset_is_columns_rows_and_opaque_sources`.
#[test]
fn a_dataset_is_columns_rows_and_opaque_sources() {
    let data = Dataset::new(
        names(&["k", "v"]),
        vec![vec![text("a"), int(1)], vec![text("b"), int(2)]],
    )
    .unwrap()
    .with_sources(sources(&[&[&["x!1"], &["y"]], &[&[], &["z"]]]))
    .unwrap()
    .with_keys(1)
    .unwrap();
    assert_eq!(data.columns(), names(&["k", "v"]));
    assert_eq!(data.sources().unwrap()[1][0], Vec::<String>::new());
    assert_eq!(data.cells(), 4);
    assert_eq!(data.column("v").unwrap(), vec![&int(1), &int(2)]);
    assert_eq!(data.records()[1], vec![("k", &text("b")), ("v", &int(2))]);
    let error = data.column("w").unwrap_err().to_json();
    assert_eq!(error["code"], "not_found");
    assert_eq!(error["valid_options"], serde_json::json!(["k", "v"]));
}

/// `test_a_malformed_dataset_is_refused` (the cases a Rust caller can write: a mapping
/// row and a source that is no string cannot be).
#[test]
fn a_malformed_dataset_is_refused() {
    assert!(Dataset::new(vec![], vec![]).is_err());
    assert!(Dataset::new(names(&["a", "a"]), vec![]).is_err());
    assert!(Dataset::new(names(&["a"]), vec![vec![int(1), int(2)]]).is_err());
    let one = || Dataset::new(names(&["a"]), vec![vec![int(1)]]).unwrap();
    assert!(one().with_sources(sources(&[&[&["s"]], &[&["t"]]])).is_err());
    assert!(Dataset::new(names(&["a"]), vec![]).unwrap().with_keys(2).is_err());
    assert!(one().with_types(vec![Some("x".into()), Some("y".into())]).is_err());
    assert_eq!(
        Dataset::new(names(&["a"]), vec![vec![int(1), int(2)]]).unwrap_err().0,
        "a row has 2 values for 1 columns"
    );
}

/// `test_a_dataset_counts_the_formula_caches_its_values_came_from`.
#[test]
fn a_dataset_counts_the_formula_caches_its_values_came_from() {
    let cache = Cache::new(&[("external", 1), ("invalidated", 0), ("cached_unverified", 2)]).unwrap();
    let data = Dataset::new(names(&["k"]), vec![vec![int(1)]])
        .unwrap()
        .with_cache(cache);
    assert_eq!(
        serde_json::Value::Object(data.cache().unwrap().to_json()).to_string(),
        r#"{"cached_unverified":2,"external":1}"#
    );
    assert!(!data.invalidated() && Cache::new(&[]).unwrap().is_none());
    let invalidated = Cache::new(&[("invalidated", 1)]).unwrap();
    assert!(Dataset::new(names(&["k"]), vec![vec![int(1)]])
        .unwrap()
        .with_cache(invalidated)
        .invalidated());
    assert_eq!(data, Dataset::new(names(&["k"]), vec![vec![int(1)]]).unwrap()); // not compared
    assert_eq!(data.row_slice(1, None).unwrap().cache(), data.cache());
    assert_eq!(
        Cache::new(&[("fresh", 1)]).unwrap_err().0,
        "a dataset's cache counts cached_unverified, invalidated, absent, external; not 'fresh'"
    );
    assert_eq!(
        Cache::new(&[("absent", -1)]).unwrap_err().0,
        "a dataset's cache count is a whole number from 0, not -1"
    );
}

/// `test_a_dataset_keeps_the_markdown_of_its_rich_text_values`.
#[test]
fn a_dataset_keeps_the_markdown_of_its_rich_text_values() {
    let rows = vec![
        vec![text("a"), text("Plain")],
        vec![text("b"), text("• one\n    • two")],
        vec![text("c"), text("Bold note")],
    ];
    let given: BTreeMap<(usize, usize), String> = [
        ((1, 1), "- one\n  - two".to_owned()),
        ((2, 1), "**Bold** note".to_owned()),
    ]
    .into();
    let plain = Dataset::new(names(&["k", "body"]), rows.clone())
        .unwrap()
        .with_keys(1)
        .unwrap();
    let data = plain.clone().with_markdown(Markdown::Given(given)).unwrap();
    assert_eq!(data.markdown_at(1, 1).as_deref(), Some("- one\n  - two"));
    assert_eq!(data.markdown_at(0, 1), None);
    assert_eq!(data.rows()[1][1], text("• one\n    • two")); // the value itself is unchanged
    assert_eq!(data, plain); // not compared
    let part = data.row_slice(2, None).unwrap();
    assert_eq!(part.markdown().unwrap().places(), vec![(0, 1), (1, 1)]);
    assert_eq!(part.markdown_at(0, 1).as_deref(), Some("- one\n  - two"));
    assert!(data.row_slice(1, Some(1)).unwrap().markdown().is_none());
    assert!(data.row_slice(1, Some(1)).unwrap().markdown_at(0, 1).is_none());
    let empty = Dataset::new(names(&["k"]), vec![vec![int(1)]])
        .unwrap()
        .with_markdown(Markdown::Given(BTreeMap::new()))
        .unwrap();
    assert!(empty.markdown().is_none());
    for place in [(3, 0), (0, 2)] {
        let bad = Markdown::Given([(place, "x".to_owned())].into());
        let made = Dataset::new(names(&["k", "v"]), vec![vec![int(1), int(2)]])
            .unwrap()
            .with_markdown(bad);
        assert!(made.is_err(), "{place:?}");
    }
    let outside = Dataset::new(names(&["k", "v"]), vec![vec![int(1), int(2)]])
        .unwrap()
        .with_markdown(Markdown::Given([((3, 0), "x".to_owned())].into()))
        .unwrap_err();
    assert_eq!(
        outside.0,
        "a dataset's markdown names (3, 0), outside its 1 rows and 2 columns (from 0)"
    );
}

/// A producer's Markdown worked out when asked for: counts what it worked out (`_Lazy`).
struct Lazy {
    places: BTreeMap<(usize, usize), Option<String>>,
    asked: Mutex<Vec<(usize, usize)>>,
}

impl LazyMarkdown for Lazy {
    fn places(&self) -> Vec<(usize, usize)> {
        self.places.keys().copied().collect()
    }

    fn markdown(&self, row: usize, column: usize) -> Option<String> {
        self.asked.lock().unwrap().push((row, column));
        self.places.get(&(row, column)).cloned().flatten()
    }
}

/// `test_a_dataset_keeps_a_lazy_markdown_and_asks_it_only_for_what_is_read`.
#[test]
fn a_dataset_keeps_a_lazy_markdown_and_asks_it_only_for_what_is_read() {
    let rows = vec![
        vec![text("a"), text("x")],
        vec![text("b"), text("• one")],
        vec![text("c"), text("y")],
        vec![text("d"), text("• two")],
    ];
    let lazy = Arc::new(Lazy {
        places: [
            ((1, 1), Some("- one".into())),
            ((2, 1), None),
            ((3, 1), Some("- two".into())),
        ]
        .into(),
        asked: Mutex::new(Vec::new()),
    });
    let asked = || lazy.asked.lock().unwrap().clone();
    let data = Dataset::new(names(&["k", "body"]), rows)
        .unwrap()
        .with_markdown(Markdown::lazy(lazy.clone()))
        .unwrap();
    assert!(asked().is_empty());
    assert_eq!(data.markdown_at(1, 1).as_deref(), Some("- one"));
    assert_eq!(data.markdown_at(2, 1), None);
    assert_eq!(data.markdown_at(0, 1), None);
    assert_eq!(asked(), vec![(1, 1), (2, 1)]);
    let part = data.row_slice(3, Some(4)).unwrap();
    let markdown = part.markdown().unwrap();
    assert_eq!(markdown.places(), vec![(0, 1), (1, 1)]);
    assert!(markdown.contains(1, 1) && !markdown.contains(1, 0) && !markdown.contains(2, 1));
    assert_eq!(asked(), vec![(1, 1), (2, 1)]); // 'in' worked nothing out
    assert_eq!(part.markdown_at(1, 1).as_deref(), Some("- two"));
    assert_eq!(part.markdown_at(0, 1), None);
    assert!(data.row_slice(1, Some(1)).unwrap().markdown().is_none());
    let outside = Arc::new(Lazy {
        places: [((1, 0), Some("x".into()))].into(),
        asked: Mutex::new(Vec::new()),
    });
    assert!(Dataset::new(names(&["k"]), vec![vec![int(1)]])
        .unwrap()
        .with_markdown(Markdown::lazy(outside))
        .is_err());
}

/// `test_an_address_names_its_id_and_the_rest_is_the_formats`.
#[test]
fn an_address_names_its_id_and_the_rest_is_the_formats() {
    assert_eq!(dataset_id("d:by_region"), Some("by_region"));
    assert_eq!(dataset_id("d:by_region[total]"), Some("by_region"));
    assert_eq!(dataset_id("d:by_region/North/total"), Some("by_region"));
    assert_eq!(dataset_id("d1"), None);
    assert_eq!(dataset_id("t:Products"), None);
}

/// `test_row_range_reads_a_run_of_rows` (sources).
#[test]
fn row_range_reads_a_run_of_rows() {
    let cases: [(&str, Option<(&str, usize, Option<usize>)>); 11] = [
        ("d:x/#9:#16", Some(("x", 9, Some(16)))),
        ("d:x/#16:#9", Some(("x", 9, Some(16)))),
        ("d:x/#9:", Some(("x", 9, None))),
        ("d:x/#9", Some(("x", 9, Some(9)))),
        ("d:x", None),
        ("d:x[a]", None),
        ("d:x/#0:#2", None),
        ("d:x/#2:3", None),
        ("d:x/North", None),
        ("d:x/#2/a", None),
        ("t:x/#2:#3", None),
    ];
    for (address, found) in cases {
        assert_eq!(row_range(address), found, "{address}");
    }
    assert_eq!(rows_text(9, Some(16)), "/#9:#16");
    assert_eq!(rows_text(9, None), "/#9:");
    assert_eq!(rows_text(9, Some(9)), "/#9");
}

/// `test_a_row_slice_keeps_sources_and_refuses_past_the_end` (sources).
#[test]
fn a_row_slice_keeps_sources_and_refuses_past_the_end() {
    let rows = vec![
        vec![text("North"), int(3), text("ok"), int(2024)],
        vec![text("South"), int(5), text("late"), int(2025)],
        vec![text("East"), Cell::Float(2.5), text(""), int(2026)],
    ];
    let letters = ["A", "B", "C", "D"];
    let cells = (0..3)
        .map(|r| letters.iter().map(|c| vec![format!("G!{c}{}", r + 2)]).collect())
        .collect();
    let dataset = Dataset::new(names(&["region", "units", "note", "year"]), rows.clone())
        .unwrap()
        .with_sources(cells)
        .unwrap()
        .with_keys(1)
        .unwrap()
        .with_types(vec![Some("text".into()); 4])
        .unwrap();
    let part = dataset.row_slice(2, None).unwrap();
    assert_eq!(part.rows(), &rows[1..]);
    assert_eq!(part.sources().unwrap()[0][0], vec!["G!A3".to_owned()]);
    assert_eq!((part.keys(), part.types()), (1, dataset.types()));
    assert_eq!(dataset.row_slice(1, Some(1)).unwrap().rows(), &rows[..1]);
    assert!(dataset.row_slice(2, Some(4)).unwrap_err().0.contains("has 3 rows"));
    assert!(dataset.row_slice(3, Some(2)).is_err());
}

/// `test_from_records_reads_mappings_by_name_in_the_order_first_met`.
#[test]
fn from_records_reads_mappings_by_name_in_the_order_first_met() {
    let march = Cell::Date(Day::new(2026, 3, 1).unwrap());
    let data = Dataset::from_records(
        Records::Maps(vec![
            vec![("region".into(), text("North")), ("opened".into(), march.clone())],
            vec![
                ("opened".into(), text("2026-04-15")),
                ("total".into(), Cell::Float(0.13)),
            ],
        ]),
        Some(Labels::ByName(vec![("opened".into(), "date".into())])),
        None,
        0,
    )
    .unwrap();
    assert_eq!(data.columns(), names(&["region", "opened", "total"]));
    assert_eq!(
        data.rows(),
        &[
            vec![text("North"), march, Cell::Blank],
            vec![Cell::Blank, text("2026-04-15"), Cell::Float(0.13)]
        ]
    );
    assert_eq!(data.types().unwrap(), &[None, Some("date".into()), None]);
    assert!(data.sources().is_none() && data.keys() == 0);
}

/// `test_from_records_takes_columns_and_a_label_per_column`.
#[test]
fn from_records_takes_columns_and_a_label_per_column() {
    let data = Dataset::from_records(
        Records::Rows(vec![vec![text("North"), int(3)], vec![text("South"), int(5)]]),
        Some(Labels::PerColumn(vec![Some("text".into()), Some("number".into())])),
        Some(names(&["region", "units"])),
        1,
    )
    .unwrap();
    assert_eq!(data.rows(), &[vec![text("North"), int(3)], vec![text("South"), int(5)]]);
    assert_eq!(data.keys(), 1);
    assert_eq!(data.types().unwrap(), &[Some("text".into()), Some("number".into())]);
    let picked = Dataset::from_records(
        Records::Maps(vec![vec![
            ("a".into(), int(1)),
            ("b".into(), int(2)),
            ("c".into(), int(3)),
        ]]),
        None,
        Some(names(&["c", "a"])),
        0,
    )
    .unwrap();
    assert_eq!(picked.columns(), names(&["c", "a"]));
    assert_eq!(picked.rows(), &[vec![int(3), int(1)]]);
}

/// `test_from_records_refuses_what_it_cannot_read` (a label given as one text, which a
/// Rust caller cannot write, aside).
#[test]
fn from_records_refuses_what_it_cannot_read() {
    let a = || Records::Maps(vec![vec![("a".into(), int(1))]]);
    let by_name = |name: &str, label: &str| Some(Labels::ByName(vec![(name.into(), label.into())]));
    let cases: [(Records, Option<Labels>, Option<Vec<String>>, &str); 7] = [
        (
            Records::Rows(vec![vec![text("North"), int(3)]]),
            None,
            None,
            "need columns",
        ),
        (a(), None, Some(names(&["a", "b"])), "no record has 'b'"),
        (a(), by_name("b", "date"), None, "'b', which is no column"),
        (a(), by_name("a", "Date"), None, "'Date' is not a type label"),
        (
            a(),
            Some(Labels::PerColumn(vec![Some("date".into()), Some("text".into())])),
            None,
            "2 type labels for 1 columns",
        ),
        (
            Records::Rows(vec![vec![text("North")]]),
            None,
            Some(names(&["a", "b"])),
            "1 values for 2 columns",
        ),
        (Records::Maps(vec![]), None, None, "give columns"),
    ];
    for (records, types, columns, words) in cases {
        let error = Dataset::from_records(records, types, columns, 0).unwrap_err();
        assert!(error.0.contains(words), "{} / {words}", error.0);
    }
}

/// `test_the_type_labels`.
#[test]
fn the_type_labels() {
    assert_eq!(
        TYPE_LABELS,
        ["number", "percent", "date", "time", "datetime", "text", "bool", "error"]
    );
}
