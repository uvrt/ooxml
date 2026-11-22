//! The record kept true per action (ooxml-edit's `_updated`).

use super::{kept, updated};
use crate::objects::chart_data::DataAction;
use ooxml_chart::edit::{record_text, with_record};
use ooxml_word_edit::RecordChange;
use ooxml_xml::{Element, Limits};
use serde_json::{json, Map, Value};
use std::sync::Arc;

fn record() -> Map<String, Value> {
    let found = json!({"v": 1, "source": "t:sales", "categories": {"column": "region"},
        "series": [{"name": "rating", "column": "b"}, {"name": "score", "column": "c"}]});
    found.as_object().cloned().unwrap_or_default()
}

fn names() -> Vec<String> {
    vec!["rating".to_owned(), "score".to_owned()]
}

fn after(action: &DataAction, names: &[String]) -> Option<Value> {
    updated(&record(), action, Some(names)).map(Value::Object)
}

fn series(found: &Value) -> Vec<&str> {
    found["series"]
        .as_array()
        .map(|list| list.iter().filter_map(|e| e["name"].as_str()).collect())
        .unwrap_or_default()
}

#[test]
fn a_category_added_or_removed_drops_the_record() {
    let add = DataAction::AddCategory {
        text: "Delta".into(),
        values: None,
    };
    assert_eq!(after(&add, &names()), None);
    assert_eq!(after(&DataAction::RemoveCategory { category: 0 }, &names()), None);
}

#[test]
fn a_category_renamed_drops_the_categories() {
    let rename = DataAction::RenameCategory {
        category: 1,
        text: "x".into(),
    };
    let found = after(&rename, &names()).unwrap_or_default();
    assert!(found.get("categories").is_none());
    assert_eq!(series(&found), ["rating", "score"]);
}

#[test]
fn values_set_drop_their_series_entry() {
    let values = DataAction::SetValues {
        series: 1,
        values: vec![Some(1.0)],
    };
    assert_eq!(series(&after(&values, &names()).unwrap_or_default()), ["rating"]);
    let value = DataAction::SetValue {
        series: 0,
        category: 0,
        value: 2.0,
    };
    let found = after(&value, &names()).unwrap_or_default();
    assert_eq!(series(&found), ["score"]);
    // the keys keep their order
    let keys: Vec<&str> = found
        .as_object()
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    assert_eq!(keys, ["v", "source", "categories", "series"]);
}

#[test]
fn a_series_renamed_or_removed_takes_its_entry_along() {
    let renamed = vec!["rating".to_owned(), "points".to_owned()];
    let rename = DataAction::RenameSeries {
        series: 1,
        text: "points".into(),
    };
    let found = after(&rename, &renamed).unwrap_or_default();
    assert_eq!(series(&found), ["rating", "points"]);
    assert_eq!(found["series"][1]["column"], "c");
    let removed = vec!["score".to_owned()];
    let found = after(&DataAction::RemoveSeries { series: 0 }, &removed).unwrap_or_default();
    assert_eq!(series(&found), ["score"]);
}

#[test]
fn nothing_left_true_drops_the_record() {
    let mut bare = record();
    bare.shift_remove("categories");
    let value = DataAction::SetValues {
        series: 0,
        values: vec![],
    };
    let one = vec!["rating".to_owned()];
    assert_eq!(updated(&bare, &value, Some(&one)), None);
    // a chart that cannot be read after the edit keeps nothing
    assert_eq!(updated(&record(), &DataAction::RemoveSeries { series: 0 }, None), None);
}

fn chart(record: Option<&str>) -> Element {
    let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>rating</c:v></c:tx></c:ser><c:ser><c:idx val="1"/><c:order val="1"/><c:tx><c:v>score</c:v></c:tx></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
    let tree = ooxml_xml::parse(xml.as_bytes(), &Limits::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let root = Arc::clone(tree.root());
    match record {
        Some(text) => with_record(&root, Some(text)).unwrap_or_else(|| (*root).clone()),
        None => (*root).clone(),
    }
}

#[test]
fn the_chart_part_and_the_documents_entry_change_together() {
    let text = super::dumps(&Value::Object(record()));
    let root = chart(Some(&text));
    let value = DataAction::SetValue {
        series: 1,
        category: 0,
        value: 3.0,
    };
    let (changed, change) = kept(&root, &value);
    let changed = changed.unwrap_or_else(|| panic!("the record changes"));
    let now = record_text(&changed).unwrap_or_default().to_owned();
    assert_eq!(change, RecordChange::Stored(now.clone()));
    assert!(now.contains(r#""series":[{"name":"rating","column":"b"}]"#), "{now}");
    let add = DataAction::AddCategory {
        text: "Delta".into(),
        values: None,
    };
    let (gone, change) = kept(&root, &add);
    assert_eq!(change, RecordChange::Dropped);
    assert_eq!(gone.as_ref().and_then(record_text), None);
    // the legend is no data: nothing changes; nor does a chart part without a record
    assert_eq!(
        kept(&root, &DataAction::SetLegend { position: None }),
        (None, RecordChange::Kept)
    );
    assert_eq!(kept(&chart(None), &add), (None, RecordChange::Kept));
}
