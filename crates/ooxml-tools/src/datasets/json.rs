//! Owns: a [`Dataset`] as JSON, both ways (design phase 3, decision D3: the application's
//! host keeps its datasets in the Rust session through a binding, and reads them back):
//! `{"columns", "rows", "sources", "keys", "types", "cache", "formats", "markdown"}`, each
//! value as [`Cell::to_json`] writes it, the Markdown a list of `[row, column, text]`.
//!
//! Must not: carry what the producer keeps for itself (its detail, which no other library
//! reads), or keep Markdown lazy across the boundary: it is worked out as it is written.

use super::{bad, Cache, Dataset, DatasetError, Markdown};
use crate::sources::Cell;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

impl Dataset {
    /// The dataset as JSON ([`Dataset::from_json`] reads it back): every field but its
    /// detail; lazy Markdown worked out now (a place that turns out to be its text left out).
    #[must_use]
    pub fn to_json(&self) -> Map<String, Value> {
        let strings = |items: &[Option<String>]| -> Value { items.iter().map(|s| Value::from(s.as_deref())).collect() };
        let mut out = Map::new();
        out.insert(
            "columns".into(),
            self.columns.iter().map(|c| Value::from(c.as_str())).collect(),
        );
        out.insert(
            "rows".into(),
            self.rows
                .iter()
                .map(|row| row.iter().map(Cell::to_json).collect::<Value>())
                .collect(),
        );
        out.insert(
            "sources".into(),
            self.sources.as_ref().map_or(Value::Null, |rows| {
                rows.iter()
                    .map(|row| {
                        row.iter()
                            .map(|cell| cell.iter().map(|s| Value::from(s.as_str())).collect::<Value>())
                            .collect::<Value>()
                    })
                    .collect()
            }),
        );
        out.insert("keys".into(), Value::from(self.keys));
        out.insert("types".into(), self.types.as_deref().map_or(Value::Null, strings));
        out.insert(
            "cache".into(),
            self.cache.as_ref().map_or(Value::Null, |c| Value::Object(c.to_json())),
        );
        out.insert("formats".into(), self.formats.as_deref().map_or(Value::Null, strings));
        out.insert(
            "markdown".into(),
            self.markdown.as_ref().map_or(Value::Null, |markdown| {
                markdown
                    .places()
                    .into_iter()
                    .filter_map(|(row, column)| {
                        let text = markdown.get(row, column)?;
                        Some(Value::from(vec![
                            Value::from(row),
                            Value::from(column),
                            Value::from(text),
                        ]))
                    })
                    .collect()
            }),
        );
        out
    }

    /// A dataset read from JSON as [`Dataset::to_json`] writes it (`sources`, `keys`,
    /// `types`, `cache`, `formats` and `markdown` may be left out or `null`).
    ///
    /// # Errors
    /// A field of the wrong shape, or a dataset its checks refuse ([`Dataset::new`] and the
    /// `with_` methods).
    pub fn from_json(value: &Value) -> Result<Dataset, DatasetError> {
        let Some(given) = value.as_object() else {
            return bad("a dataset is a JSON object");
        };
        let present = |key: &str| given.get(key).filter(|v| !v.is_null());
        let columns = texts(given.get("columns"), "columns")?;
        let rows = given
            .get("rows")
            .and_then(Value::as_array)
            .ok_or_else(|| DatasetError("a dataset's rows are a list of lists".into()))?
            .iter()
            .map(|row| {
                row.as_array()
                    .and_then(|cells| cells.iter().map(Cell::from_json).collect::<Option<Vec<Cell>>>())
                    .ok_or_else(|| DatasetError("a dataset's row is a list of values".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut dataset = Dataset::new(columns, rows)?;
        if let Some(sources) = present("sources") {
            dataset = dataset.with_sources(sources_of(sources)?)?;
        }
        if let Some(keys) = present("keys") {
            let keys = keys
                .as_u64()
                .and_then(|k| usize::try_from(k).ok())
                .ok_or_else(|| DatasetError("a dataset's keys is a whole number from 0".into()))?;
            dataset = dataset.with_keys(keys)?;
        }
        if let Some(types) = present("types") {
            dataset = dataset.with_types(optional_texts(types, "types")?)?;
        }
        if let Some(cache) = present("cache") {
            let counts = cache
                .as_object()
                .ok_or_else(|| DatasetError("a dataset's cache is a mapping of counts".into()))?
                .iter()
                .map(|(status, n)| {
                    n.as_i64()
                        .map(|n| (status.as_str(), n))
                        .ok_or_else(|| DatasetError("a dataset's cache counts are whole numbers".into()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            dataset = dataset.with_cache(Cache::new(&counts)?);
        }
        if let Some(formats) = present("formats") {
            dataset = dataset.with_formats(optional_texts(formats, "formats")?)?;
        }
        if let Some(markdown) = present("markdown") {
            dataset = dataset.with_markdown(markdown_of(markdown)?)?;
        }
        Ok(dataset)
    }
}

/// Each value's sources: a list per row with a list of text per column.
fn sources_of(value: &Value) -> Result<Vec<Vec<Vec<String>>>, DatasetError> {
    let rows = || DatasetError("a dataset's sources are a list of rows".into());
    value
        .as_array()
        .ok_or_else(rows)?
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or_else(rows)?
                .iter()
                .map(|cell| texts(Some(cell), "sources"))
                .collect()
        })
        .collect()
}

/// The Markdown of a dataset's values: a list of `[row, column, text]`.
fn markdown_of(value: &Value) -> Result<Markdown, DatasetError> {
    let shape = || DatasetError("a dataset's markdown is a list of [row, column, text]".into());
    let index = |place: &Value, at: usize| {
        place
            .get(at)
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
    };
    let mut found = BTreeMap::new();
    for place in value.as_array().ok_or_else(shape)? {
        let (Some(row), Some(column), Some(text)) =
            (index(place, 0), index(place, 1), place.get(2).and_then(Value::as_str))
        else {
            return Err(shape());
        };
        found.insert((row, column), text.to_owned());
    }
    Ok(Markdown::Given(found))
}

/// A list of text, or a refusal naming `what`.
fn texts(value: Option<&Value>, what: &str) -> Result<Vec<String>, DatasetError> {
    value
        .and_then(Value::as_array)
        .and_then(|items| items.iter().map(|v| v.as_str().map(str::to_owned)).collect())
        .ok_or_else(|| DatasetError(format!("a dataset's {what} are lists of text")))
}

/// A list of text or `null`, or a refusal naming `what`.
fn optional_texts(value: &Value, what: &str) -> Result<Vec<Option<String>>, DatasetError> {
    value
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .map(|v| match v {
                    Value::Null => Some(None),
                    Value::String(s) => Some(Some(s.clone())),
                    _ => None,
                })
                .collect()
        })
        .ok_or_else(|| DatasetError(format!("a dataset's {what} are a list of text or null, one per column")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_dataset_reads_back_as_it_was_written() {
        let given = json!({
            "columns": ["topic", "n"], "rows": [["door", 2], ["roof", {"date": [2026, 1, 5]}]],
            "sources": [[["A1"], ["B1"]], [["A2"], []]], "keys": 1, "types": [null, "number"],
            "cache": {"invalidated": 1}, "formats": [null, "0.0"], "markdown": [[0, 0, "**door**"]]});
        let dataset = Dataset::from_json(&given).unwrap();
        assert_eq!(dataset.keys(), 1);
        assert_eq!(dataset.markdown_at(0, 0).as_deref(), Some("**door**"));
        assert!(dataset.invalidated());
        assert_eq!(Value::Object(dataset.to_json()), given);
        let again = Dataset::from_json(&Value::Object(dataset.to_json())).unwrap();
        assert_eq!(again, dataset);
        assert_eq!(again.formats(), dataset.formats());
    }

    #[test]
    fn a_bare_dataset_needs_only_columns_and_rows() {
        let dataset = Dataset::from_json(&json!({"columns": ["a"], "rows": [[1.5], [null]]})).unwrap();
        assert_eq!(dataset.rows(), &[vec![Cell::Float(1.5)], vec![Cell::Blank]]);
        assert!(dataset.markdown().is_none() && dataset.sources().is_none());
    }

    #[test]
    fn a_dataset_of_the_wrong_shape_is_refused_with_why() {
        let refused = |v: Value| Dataset::from_json(&v).unwrap_err().0;
        assert!(refused(json!({"columns": ["a"], "rows": [[1, 2]]})).contains("2 values for 1 columns"));
        assert!(refused(json!({"columns": ["a"], "rows": [[[1]]]})).contains("a list of values"));
        assert!(refused(json!({"columns": ["a"], "rows": [], "markdown": [[0, 0]]})).contains("[row, column, text]"));
        assert!(refused(json!([])).contains("JSON object"));
    }
}
