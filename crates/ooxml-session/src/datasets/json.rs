//! Owns: a kept dataset as JSON, both ways (design phase 3, decision D3): what the
//! application's own host (the Python host of pptx and xlsx, until Phase 4) hands the
//! session's store when its producers keep a dataset, and what it reads back -- the entry's
//! id, origin, time and [`Derivation`] (`{"id", "doc", "kind", "name", "version", "closed",
//! "for_good", "invalidated", "also"}`), its values as [`Dataset::to_json`] writes them.
//!
//! Must not: say whether an entry is stale (the store does, against the heads now).

use super::{DatasetEntry, Derivation};
use crate::DocKind;
use ooxml_tools::datasets::{check_dataset_id, Dataset};
use ooxml_tools::TryCall;
use serde_json::{Map, Value};
use std::sync::Arc;

impl Derivation {
    /// Where the values came from, as JSON ([`Derivation::from_json`] reads it back).
    #[must_use]
    pub fn to_json(&self) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("id".into(), Value::from(self.id.as_str()));
        out.insert("doc".into(), Value::from(self.doc.as_deref()));
        out.insert("kind".into(), Value::from(self.kind.map(DocKind::as_str)));
        out.insert("name".into(), Value::from(self.name.as_deref()));
        out.insert("version".into(), Value::from(self.version));
        out.insert("closed".into(), Value::Bool(self.closed));
        out.insert("for_good".into(), Value::Bool(self.for_good));
        out.insert("invalidated".into(), Value::from(self.invalidated));
        out.insert(
            "also".into(),
            self.also.iter().map(|other| Value::Object(other.to_json())).collect(),
        );
        out
    }

    /// Where the values came from, read from JSON as [`Derivation::to_json`] writes it (all
    /// but `id` may be left out). A kind no Rust format holds is kept as none: only the
    /// application's own host reads its documents.
    ///
    /// # Errors
    /// A field of the wrong shape.
    pub fn from_json(value: &Value) -> Result<Derivation, String> {
        let given = value.as_object().ok_or("a dataset's source is a JSON object")?;
        let text = |key: &str| -> Result<Option<String>, String> {
            match given.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err(format!("a dataset's source's {key} is text")),
            }
        };
        let flag = |key: &str| given.get(key).and_then(Value::as_bool).unwrap_or(false);
        let id = text("id")?.ok_or("a dataset's source names its dataset's id")?;
        let version = match given.get("version") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_u64().ok_or("a dataset's source's version is a whole number")?),
        };
        let also = match given.get("also") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items.iter().map(Derivation::from_json).collect::<Result<_, _>>()?,
            Some(_) => return Err("a dataset's source's also is a list".into()),
        };
        Ok(Derivation {
            id,
            doc: text("doc")?,
            kind: text("kind")?.as_deref().and_then(DocKind::parse),
            name: text("name")?,
            version,
            closed: flag("closed"),
            for_good: flag("for_good"),
            invalidated: given.get("invalidated").and_then(Value::as_u64).unwrap_or(0),
            also,
        })
    }
}

impl DatasetEntry {
    /// The entry but its values, as JSON: `{"id", "origin", "defined_at", "source"}`.
    #[must_use]
    pub fn meta_json(&self) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("id".into(), Value::from(self.id.as_str()));
        out.insert("origin".into(), Value::from(self.origin.as_deref()));
        out.insert("defined_at".into(), Value::from(self.defined_at.as_deref()));
        out.insert("source".into(), Value::Object(self.source.to_json()));
        out
    }

    /// An entry the application's own host kept, read from JSON: [`DatasetEntry::meta_json`]'s
    /// fields and `dataset` ([`Dataset::from_json`]). `source` may be left out: no document.
    ///
    /// # Errors
    /// An id of the wrong shape, or a field of the wrong shape.
    pub fn from_json(value: &Value) -> Result<DatasetEntry, String> {
        let given = value.as_object().ok_or("a dataset entry is a JSON object")?;
        let id = given
            .get("id")
            .and_then(Value::as_str)
            .ok_or("a dataset entry has an id")?;
        let retry = TryCall {
            tool: "keep_datasets".into(),
            arguments: Map::new(),
            fill: Vec::new(),
        };
        let id = check_dataset_id(id, "id", &retry).map_err(|e| format!("{id:?}: {}", e.message()))?;
        let dataset = Dataset::from_json(given.get("dataset").unwrap_or(&Value::Null)).map_err(|e| e.0)?;
        let text = |key: &str| given.get(key).and_then(Value::as_str).map(str::to_owned);
        let mut entry = DatasetEntry::new(id, Arc::new(dataset), None);
        if let Some(source) = given.get("source").filter(|s| !s.is_null()) {
            entry.source = Derivation::from_json(source)?;
            entry.source.id = id.to_owned();
        }
        entry.origin = text("origin");
        entry.defined_at = text("defined_at");
        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn heads(doc: &str) -> Option<u64> {
        (doc == "d2").then_some(5)
    }

    #[test]
    fn an_entry_reads_back_with_its_source_and_fate() {
        let given = json!({
            "id": "by_region", "origin": "t:Sales", "defined_at": "2026-11-27T10:00:00Z",
            "dataset": {"columns": ["Region"], "rows": [["North"]]},
            "source": {"id": "x", "doc": "d2", "kind": "xlsx", "name": "sales.xlsx", "version": 4,
                       "also": [{"id": "by_region", "doc": "d3", "closed": true, "for_good": true}]}});
        let entry = DatasetEntry::from_json(&given).unwrap();
        assert_eq!(entry.doc(), Some("d2"));
        assert_eq!(entry.source.kind, Some(DocKind::Xlsx));
        assert!(
            entry.stale(&heads),
            "d2 is at 5, the dataset of 4; and d3 was stale when it closed"
        );
        let mut meta = entry.meta_json();
        assert_eq!(meta["source"]["id"], "by_region");
        meta.insert("dataset".into(), Value::Object(entry.dataset.to_json()));
        assert_eq!(DatasetEntry::from_json(&Value::Object(meta)).unwrap(), entry);
    }

    #[test]
    fn an_entry_without_a_source_is_the_applications_and_never_stale() {
        let entry =
            DatasetEntry::from_json(&json!({"id": "notes", "dataset": {"columns": ["a"], "rows": []}})).unwrap();
        assert_eq!(entry.doc(), None);
        assert!(!entry.stale(&heads));
        assert!(entry.brief(false)["from"] == "application");
    }

    #[test]
    fn a_dataset_kept_again_or_dropped_and_kept_again_has_another_serial() {
        use super::super::{DatasetLimits, DatasetStore};
        let entry = |rows: Value| {
            DatasetEntry::from_json(&json!({"id": "notes", "dataset": {"columns": ["a"], "rows": rows}})).unwrap()
        };
        let mut store = DatasetStore::new(DatasetLimits::default());
        store.keep(vec![entry(json!([[1]]))], &heads).unwrap();
        let first = store.serial("notes").unwrap();
        store.keep(vec![entry(json!([[2]]))], &heads).unwrap();
        let second = store.serial("notes").unwrap();
        assert_ne!(first, second);
        let retry = TryCall {
            tool: "t".into(),
            arguments: Map::new(),
            fill: Vec::new(),
        };
        store.drop_dataset("d:notes", &retry).unwrap();
        assert_eq!(store.serial("notes"), None);
        store.keep(vec![entry(json!([[2]]))], &heads).unwrap();
        assert!(![first, second].contains(&store.serial("notes").unwrap()));
        let before = store.serial("notes");
        let _ = store.find_mut("notes");
        assert_ne!(store.serial("notes"), before, "changed in place: kept anew");
    }

    #[test]
    fn an_entry_of_the_wrong_shape_is_refused_with_why() {
        let refused = |v: Value| DatasetEntry::from_json(&v).unwrap_err();
        assert!(refused(json!({"id": "1x", "dataset": {"columns": ["a"], "rows": []}})).contains("1x"));
        assert!(refused(json!({"id": "x", "dataset": {"columns": [], "rows": []}})).contains("at least one column"));
        assert!(refused(
            json!({"id": "x", "dataset": {"columns": ["a"], "rows": []}, "source": {"id": "x",
                               "version": "4"}})
        )
        .contains("whole number"));
    }
}
