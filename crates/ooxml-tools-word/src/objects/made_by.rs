//! Owns: [`MadeBy`], the host's record of the session's `word_edit_table` calls that left a
//! table ragged once Accept All runs or off its grid (docx-agent's `_ragged_by` and
//! `_off_grid_by`, rounds 57 and 58, R125 and R128): each table's call, kept by table id,
//! which `check`'s `table_grid_mismatch` names as an entry's `made_by` ([`MadeBy::add_to`]).
//!
//! Must not: find which tables are ragged or off their grid (the facts do), or prune: an
//! entry stays until its table is no longer found so (an undone call's table is simply not
//! listed again).

use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

/// How a call left a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Its rows end on different grid columns once its tracked changes are accepted (or
    /// now): `table_grid_mismatch`'s `drawn: ragged`.
    Ragged,
    /// A row's cells do not reach exactly across its grid: `drawn: widened` or `short`.
    OffGrid,
}

/// The calls that left tables ragged or off their grid, by table id: `{"tool",
/// "arguments"}` each, the arguments as the call gave them (none left out but nulls).
/// Writers on several threads add to it at once.
#[derive(Debug, Default)]
pub struct MadeBy {
    calls: Mutex<Calls>,
}

#[derive(Debug, Default)]
struct Calls {
    ragged: BTreeMap<String, Value>,
    off_grid: BTreeMap<String, Value>,
}

impl Calls {
    fn of(&mut self, shape: Shape) -> &mut BTreeMap<String, Value> {
        match shape {
            Shape::Ragged => &mut self.ragged,
            Shape::OffGrid => &mut self.off_grid,
        }
    }
}

impl MadeBy {
    /// Keep that `tool`, called with `arguments`, left `table` so.
    pub fn note(&self, shape: Shape, table: &str, tool: &str, arguments: &Map<String, Value>) {
        let given: Map<String, Value> = arguments
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut call = Map::new();
        call.insert("tool".into(), Value::from(tool));
        call.insert("arguments".into(), Value::Object(given));
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.of(shape).insert(table.to_owned(), Value::Object(call));
    }

    /// The table `from` is named `to` now (a row inserted or deleted at its top): what was
    /// kept of it goes with it.
    pub fn renamed(&self, from: &str, to: &str) {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        for shape in [Shape::Ragged, Shape::OffGrid] {
            let kept = calls.of(shape);
            if let Some(call) = kept.remove(from) {
                kept.insert(to.to_owned(), call);
            }
        }
    }

    /// The call kept for `table` left `shape`, if any.
    #[must_use]
    pub fn of(&self, shape: Shape, table: &str) -> Option<Value> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.of(shape).get(table).cloned()
    }

    /// Each `table_grid_mismatch` entry of `entries` given `made_by` (last), the call this
    /// session kept for its table drawn so, where one was kept.
    pub fn add_to(&self, entries: &mut [Value]) {
        for entry in entries {
            let Value::Object(map) = entry else { continue };
            let shape = match map.get("drawn").and_then(Value::as_str) {
                Some("ragged") => Shape::Ragged,
                _ => Shape::OffGrid,
            };
            let made = map.get("table").and_then(Value::as_str).and_then(|t| self.of(shape, t));
            if let Some(made) = made {
                map.insert("made_by".into(), made);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MadeBy, Shape};
    use serde_json::{json, Value};

    #[test]
    fn an_entry_names_the_call_kept_for_its_table_drawn_so() {
        let made = MadeBy::default();
        let arguments = json!({"doc": "d1", "table": "t:1", "action": "delete_column", "index": 3, "side": null});
        made.note(
            Shape::Ragged,
            "t:1",
            "word_edit_table",
            arguments.as_object().unwrap_or(&Default::default()),
        );
        made.renamed("t:1", "t:2");
        let mut entries = vec![
            json!({"table": "t:2", "drawn": "ragged"}),
            json!({"table": "t:2", "drawn": "widened"}),
            Value::from("x"),
        ];
        made.add_to(&mut entries);
        assert_eq!(
            entries[0]["made_by"],
            json!({"tool": "word_edit_table", "arguments": {"doc": "d1", "table": "t:1", "action": "delete_column", "index": 3}})
        );
        assert!(entries[1].get("made_by").is_none(), "off its grid: no call kept");
    }
}
