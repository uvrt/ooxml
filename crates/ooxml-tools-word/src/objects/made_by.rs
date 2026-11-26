//! Owns: [`MadeBy`], the host's record of the session's `word_edit_table` calls that left a
//! table ragged once Accept All runs or off its grid (docx-agent's `_ragged_by` and
//! `_off_grid_by`, rounds 57 and 58, R125 and R128): each table's call, kept by table id,
//! which `check`'s `table_grid_mismatch` names as an entry's `made_by` ([`MadeBy::add_to`]).
//!
//! An op of a host's batch keeps what it notes in a copy for its attempt
//! ([`MadeBy::attempt`]), applied to the document's once the batch is published
//! ([`MadeBy::apply`]): an attempt the batch prepares again leaves nothing.
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
    /// What an attempt's copy was told, in order, for the document's to be told the same
    /// once the batch is published; `None`: the document's own.
    told: Option<Mutex<Vec<Told>>>,
}

/// What a [`MadeBy`] was told.
#[derive(Debug, Clone)]
enum Told {
    Noted(Shape, String, Value),
    Renamed(String, String),
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
    /// A copy of it for an attempt of a host's batch: it reads as this one and keeps what it
    /// is told for [`MadeBy::apply`].
    #[must_use]
    pub fn attempt(&self) -> MadeBy {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        MadeBy {
            calls: Mutex::new(Calls {
                ragged: calls.ragged.clone(),
                off_grid: calls.off_grid.clone(),
            }),
            told: Some(Mutex::new(Vec::new())),
        }
    }

    /// Tell it what `attempt` ([`MadeBy::attempt`]) was told, in order.
    pub fn apply(&self, attempt: &MadeBy) {
        let told = attempt
            .told
            .as_ref()
            .map(|t| t.lock().unwrap_or_else(PoisonError::into_inner).clone())
            .unwrap_or_default();
        for told in told {
            match told {
                Told::Noted(shape, table, call) => self.keep(shape, &table, call),
                Told::Renamed(from, to) => self.renamed(&from, &to),
            }
        }
    }

    /// What it is told, kept where it is an attempt's copy.
    fn told(&self, told: Told) {
        if let Some(kept) = &self.told {
            kept.lock().unwrap_or_else(PoisonError::into_inner).push(told);
        }
    }

    /// Keep `call` as the one that left `table` so.
    fn keep(&self, shape: Shape, table: &str, call: Value) {
        self.told(Told::Noted(shape, table.to_owned(), call.clone()));
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.of(shape).insert(table.to_owned(), call);
    }

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
        self.keep(shape, table, Value::Object(call));
    }

    /// The table `from` is named `to` now (a row inserted or deleted at its top): what was
    /// kept of it goes with it.
    pub fn renamed(&self, from: &str, to: &str) {
        self.told(Told::Renamed(from.to_owned(), to.to_owned()));
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

    #[test]
    fn an_attempt_reads_as_the_document_and_is_applied_once_in_order() {
        let made = MadeBy::default();
        let arguments = json!({"doc": "d1"});
        let arguments = arguments.as_object().unwrap_or_else(|| unreachable!());
        made.note(Shape::Ragged, "t:1", "word_edit_table", arguments);
        let attempt = made.attempt();
        assert!(attempt.of(Shape::Ragged, "t:1").is_some(), "reads as the document's");
        attempt.renamed("t:1", "t:2");
        attempt.note(Shape::OffGrid, "t:3", "word_format_table", arguments);
        assert!(
            made.of(Shape::Ragged, "t:2").is_none(),
            "the document's untouched meanwhile"
        );
        // an attempt prepared again is dropped: only the one published is applied
        drop(made.attempt());
        made.apply(&attempt);
        assert!(made.of(Shape::Ragged, "t:1").is_none());
        assert!(made.of(Shape::Ragged, "t:2").is_some());
        assert!(made.of(Shape::OffGrid, "t:3").is_some());
    }
}
