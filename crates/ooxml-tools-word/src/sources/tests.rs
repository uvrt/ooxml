use super::label::date_value;
use super::*;
use ooxml_numfmt::{Locale, EN_US};
use serde_json::json;

fn dutch() -> Locale {
    Locale::new(',', '.', "dmy", "nl-NL", ';').unwrap()
}

#[test]
fn values_are_written_as_a_category_label_is() {
    let text = |v: SourceValue, kind: Option<&str>| label(&v, &EN_US, kind, None);
    assert_eq!(text(SourceValue::Float(1.5), None).as_deref(), Some("1.5"));
    assert_eq!(
        label(&SourceValue::Float(1.5), &dutch(), None, None).as_deref(),
        Some("1,5")
    );
    assert_eq!(text(SourceValue::Float(2024.0), None).as_deref(), Some("2024"));
    assert_eq!(text(SourceValue::Float(0.1 + 0.2), None).as_deref(), Some("0.3"));
    assert_eq!(text(SourceValue::Float(1e20), None).as_deref(), Some("1e+20"));
    assert_eq!(text(SourceValue::Int("12".into()), None).as_deref(), Some("12"));
    assert_eq!(text(SourceValue::Bool(true), None).as_deref(), Some("TRUE"));
    assert_eq!(text(SourceValue::Text(String::new()), None), None);
    let day = Day::new(2026, 3, 1).unwrap();
    assert_eq!(text(SourceValue::Date(day), None).as_deref(), Some("3/1/2026"));
    assert_eq!(
        label(&SourceValue::Date(day), &dutch(), None, None).as_deref(),
        Some("1/3/2026")
    );
    let clock = Clock::new(14, 5, 0, 0).unwrap();
    assert_eq!(
        text(SourceValue::DateTime(day, clock), None).as_deref(),
        Some("3/1/2026 14:05")
    );
    // ISO text is a date in a date column only
    assert_eq!(
        text(SourceValue::Text("2026-03-01".into()), Some("date")).as_deref(),
        Some("3/1/2026")
    );
    assert_eq!(
        text(SourceValue::Text("2026-03-01".into()), None).as_deref(),
        Some("2026-03-01")
    );
    assert_eq!(
        text(SourceValue::Text("14:05:30.250".into()), Some("time")).as_deref(),
        Some("14:05:30")
    );
    assert_eq!(
        text(SourceValue::Text("PT25H".into()), Some("time")).as_deref(),
        Some("PT25H")
    );
    assert_eq!(
        text(SourceValue::Text("2026-02-30".into()), Some("date")).as_deref(),
        Some("2026-02-30")
    );
    assert_eq!(text(SourceValue::Other("a Decimal".into()), None), None);
}

#[test]
fn a_date_pattern_writes_measured_names() {
    let pattern = date_pattern("d mmmm yyyy", &EN_US, "date_format").unwrap();
    assert_eq!(pattern.format(Day::new(2026, 3, 2).unwrap()), "2 March 2026");
    let dutch_pattern = date_pattern("dddd d mmm", &dutch(), "date_format").unwrap();
    assert_eq!(dutch_pattern.format(Day::new(2026, 3, 2).unwrap()), "maandag 2 mrt");
    let fixed = date_pattern("dd/mm/yy \"w\" \\x", &EN_US, "date_format").unwrap();
    assert_eq!(fixed.format(Day::new(2026, 3, 2).unwrap()), "02/03/26 w x");
    for bad in ["", "ddddd", "d h", "\"open"] {
        let error = date_pattern(bad, &EN_US, "date_format").unwrap_err();
        assert_eq!(error.field(), Some("date_format"), "{bad}");
    }
    let japanese = Locale::new('.', ',', "ymd", "ja-JP", ',').unwrap();
    assert!(date_pattern("d mmmm", &japanese, "date_format").is_err());
    assert!(date_pattern("dd/mm", &japanese, "date_format").is_ok());
    assert_eq!(Day::new(2026, 11, 22).unwrap().weekday(), 6);
    assert_eq!(
        date_value(&SourceValue::Text("2026-03-01T10:00".into()), Some("datetime")),
        Day::new(2026, 3, 1)
    );
}

#[test]
fn row_ranges_and_runs_are_written_as_ooxml_edit_writes_them() {
    assert_eq!(first_rows("t:sales", 9).as_deref(), Some("t:sales/#1:#9"));
    assert_eq!(
        first_rows("d:by_region/#10:", 5).as_deref(),
        Some("d:by_region/#10:#14")
    );
    assert_eq!(first_rows("d2/t:sales", 1).as_deref(), Some("d2/t:sales/#1"));
    assert_eq!(first_rows("d:x/#2:#40", 3).as_deref(), Some("d:x/#2:#4"));
    assert_eq!(first_rows("d:x/a", 3), None);
    assert_eq!(first_rows("d:x", 0), None);
    assert_eq!(runs(&[0, 1, 2, 3, 6]), "1-4,7");
    assert_eq!(runs(&[]), "");
}

#[test]
fn rows_are_read_from_the_hosts_json() {
    let given = json!([{
        "given": "d:sales", "address": "d:sales", "columns": ["region", "when", "n"],
        "types": [null, "date", null],
        "rows": [["North", {"date": [2026, 1, 20]}, 3], [null, "2026-01-21", 2.5]],
        "sources": [[["A2"], ["B2"], ["C2"]], [["A3"], ["B3"], ["C3"]]],
        "doc": "d2", "name": "sales.xlsx", "version": 4, "stale": false, "cache": null
    }]);
    let rows = Sources::from_json(&given).unwrap().rows.remove(0);
    assert_eq!(rows.value(0, 1), &SourceValue::Date(Day::new(2026, 1, 20).unwrap()));
    assert_eq!(rows.value(1, 2), &SourceValue::Float(2.5));
    assert_eq!(rows.where_is(1, 2), "C3 (row 2 of d:sales, column 'n')");
    let retry = ooxml_tools::TryCall {
        tool: "t".into(),
        arguments: serde_json::Map::new(),
        fill: Vec::new(),
    };
    assert_eq!(rows.column("REGION", "from_columns[0]", &retry).unwrap(), 0);
    assert!(rows.column("nothing", "from_columns[0]", &retry).is_err());
    let record = rows.provenance(2, &[0, 1]);
    assert_eq!(
        Value::Object(record),
        json!({"column": "n", "rows": "1-2", "sources": ["C2", "C3"]})
    );
}
