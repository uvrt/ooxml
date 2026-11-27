//! Owns: a chart's record of where its values came from kept true by `edit_chart`'s data
//! actions given no `from` (ooxml-edit's `RECORD_ACTIONS`, `_edit_record` and `_updated` in
//! `tools/shared.py`): read from the chart part as the edit left it, then -- `add_category`
//! and `remove_category` change every series' rows, so the record goes; `rename_category`
//! drops its categories; `set_values` and `set_value` drop the changed series' entry;
//! `rename_series` renames the entry the series had; `remove_series` drops it; and whatever
//! the action, an entry naming a series the chart no longer has goes -- the record gone
//! when neither series nor categories are left. A changed record is written into the chart
//! part and into the document's records (in the edit's step); an unchanged one is left.
//!
//! The changed series is the one the call resolved (`names`: by name, loosely, or by
//! position), where ooxml-edit compares the `series` given with the names as they stand.
//!
//! Must not: change the chart's data, or keep a record where the chart part has none.

use crate::data::DataAction;
use ooxml_base::pyfmt::repr_float;
use ooxml_chart::data::{chart_data, Point};
use ooxml_chart::edit::RecordChange;
use ooxml_chart::edit::{record_text, with_record};
use ooxml_provenance::{dumps, record};
use ooxml_xml::Element;
use serde_json::{Map, Value};

/// The chart's record after `action`, given the chart part's root as the action left it: the
/// root with the record changed (`None`: as it is) and the document's entry's change.
pub(crate) fn kept(after: &Element, action: &DataAction) -> (Option<Element>, RecordChange) {
    let unchanged = (None, RecordChange::Kept);
    if !changes_data(action) {
        return unchanged;
    }
    let Some(before) = record_text(after).and_then(record) else {
        return unchanged;
    };
    let names = chart_data(after).ok().map(|data| {
        data.series
            .iter()
            .map(|s| s.name.as_ref().map_or_else(|| "None".to_owned(), Point::text))
            .collect::<Vec<_>>()
    });
    let updated = updated(&before, action, names.as_deref());
    if updated.as_ref() == Some(&before) {
        return unchanged;
    }
    let text = updated.map(|found| dumps(&Value::Object(found)));
    let root = with_record(after, text.as_deref());
    (root, text.map_or(RecordChange::Dropped, RecordChange::Stored))
}

/// Whether `action` is one of `RECORD_ACTIONS`: it changes the values a record says.
fn changes_data(action: &DataAction) -> bool {
    matches!(
        action,
        DataAction::SetValues { .. }
            | DataAction::SetValue { .. }
            | DataAction::AddCategory { .. }
            | DataAction::RemoveCategory { .. }
            | DataAction::RenameCategory { .. }
            | DataAction::RemoveSeries { .. }
            | DataAction::RenameSeries { .. }
    )
}

/// `before` after `action` (`_updated`), `names` the chart's series names after it (`None`
/// where the chart cannot be read: nothing of the record is known true); `None` when nothing
/// of it stays true.
fn updated(before: &Map<String, Value>, action: &DataAction, names: Option<&[String]>) -> Option<Map<String, Value>> {
    let names = names?;
    if matches!(
        action,
        DataAction::AddCategory { .. } | DataAction::RemoveCategory { .. }
    ) {
        return None;
    }
    let mut after = before.clone();
    let mut entries: Vec<Map<String, Value>> = before
        .get("series")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_object).cloned().collect())
        .unwrap_or_default();
    match action {
        DataAction::RenameCategory { .. } => {
            after.shift_remove("categories");
        }
        DataAction::SetValues { series, .. } | DataAction::SetValue { series, .. } => {
            if let Some(changed) = names.get(*series) {
                entries.retain(|entry| entry_name(entry) != *changed);
            }
        }
        DataAction::RenameSeries { text, .. } => {
            let mut gone: Vec<&mut Map<String, Value>> = entries
                .iter_mut()
                .filter(|entry| !names.contains(&entry_name(entry)))
                .collect();
            if gone.len() == 1 && names.contains(text) {
                if let Some(entry) = gone.pop() {
                    entry.insert("name".into(), Value::from(text.as_str()));
                }
            }
        }
        _ => {}
    }
    // Whatever the edit, an entry names a series the chart has.
    entries.retain(|entry| names.contains(&entry_name(entry)));
    if entries.is_empty() {
        after.shift_remove("series");
    } else {
        after.insert(
            "series".into(),
            Value::Array(entries.into_iter().map(Value::Object).collect()),
        );
    }
    (after.contains_key("series") || after.contains_key("categories")).then_some(after)
}

/// An entry's series name as Python's `str` gives it.
fn entry_name(entry: &Map<String, Value>) -> String {
    match entry.get("name") {
        None | Some(Value::Null) => "None".to_owned(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(true)) => "True".to_owned(),
        Some(Value::Bool(false)) => "False".to_owned(),
        Some(Value::Number(n)) if n.is_f64() => repr_float(n.as_f64().unwrap_or(0.0)),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;
