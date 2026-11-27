//! Owns: a table's record of where its values came from kept true through a change made
//! without a source (docx-agent's `tools/objects.py`, `_Kept`): [`KeepingRecord`] runs a
//! change of the table's rows or columns ([`EditTable`]) or a write of its cells
//! ([`SetCells`]) and then, in the same transaction, the record as the change leaves it --
//! a row added or deleted below the block leaves it, above it moves it, inside it (or at its
//! header) drops it; a column added or deleted moves the columns after it, a column deleted
//! takes its own; a cell written inside the block takes its column out; a merge or a split
//! drops it; no column left drops it -- stored under the table's durable id after the
//! change (the entry under the id it had before dropped where the change renamed it).
//!
//! Positions count as accepted, as the record does: rows whose deletion is pending are not
//! counted, columns by their place once Accept All has run.
//!
//! Must not: decide where a value goes, or change a table without a record otherwise than
//! the change it runs does.

use super::rows::{EditTable, Placing, TableChange, TableChanged};
use super::{find_table, Found};
use crate::policy::RevisionPolicy;
use crate::set_cells::{found_table, CellsSet, SetCells};
use crate::sink::EditSink;
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_provenance::dumps;
use ooxml_word_model::docx::provenance::{kind_of, Kind};
use ooxml_word_model::docx::tables::{Accepted, TableGrid};
use serde_json::{Map, Value};

/// A change of a table that keeps its record of its source true (docx-agent's `_Kept`):
/// [`EditTable`] or [`SetCells`].
#[derive(Debug, Clone)]
pub struct KeepingRecord<C>(pub C);

/// The table's record as the change found it.
struct Before {
    /// The table's durable id.
    key: String,
    /// Its record.
    record: Map<String, Value>,
    /// The table as accepted.
    accepted: Accepted,
}

impl Before {
    /// The record of the table `target` names, where it has one (read through `sink`).
    fn read(sink: &mut EditSink<'_, '_>, target: &str) -> Result<Option<Before>> {
        let docx = sink.docx()?;
        let Found { id, .. } = find_table(sink.doc(), &docx, target)?;
        if kind_of(&id) != Some(Kind::Table) {
            return Ok(None);
        }
        let Some(record) = crate::provenance::table_record(sink, &id)? else {
            return Ok(None);
        };
        let (story, table) = found_table(sink.doc(), &docx, target)?;
        let accepted = TableGrid::new(story, &table).accepted(&table.id);
        Ok(Some(Before {
            key: id,
            record,
            accepted,
        }))
    }

    /// The block's first and last row as accepted, its header row included (`_block`).
    fn block(&self) -> (i64, i64) {
        let first = number(&self.record, "first_row").unwrap_or(0);
        let top = number(&self.record, "header_row").unwrap_or(first);
        let rows = columns(&self.record)
            .iter()
            .map(|c| count(c.get("rows").and_then(Value::as_str).unwrap_or_default()))
            .max()
            .unwrap_or(0);
        (top, top.max(first + rows - 1))
    }

    /// Where `row` (a document row) is among the rows as accepted before.
    fn accepted_index(&self, row: usize) -> Option<i64> {
        let at = self.accepted.rows.iter().position(|r| *r == row)?;
        i64::try_from(at).ok()
    }

    /// The record once cells at `cells` (document row, grid column) are written: the
    /// columns written inside the block taken out.
    fn after_cells(&self, cells: &[(usize, usize)]) -> Option<Map<String, Value>> {
        let (top, last) = self.block();
        let dropped: Vec<i64> = cells
            .iter()
            .filter(|(row, _)| self.accepted_index(*row).is_some_and(|at| (top..=last).contains(&at)))
            .filter_map(|(_, column)| i64::try_from(self.accepted.from_grid(*column)).ok())
            .collect();
        let kept = columns(&self.record)
            .into_iter()
            .filter(|c| !number(c, "col").is_some_and(|col| dropped.contains(&col)))
            .collect();
        self.with_columns(kept)
    }

    /// The record once `change` is made; `rows_now`, the rows as accepted after it.
    fn after_change(&self, change: TableChange, rows_now: &[usize]) -> Option<Map<String, Value>> {
        let (top, last) = self.block();
        let total = self.accepted.total_rows;
        let step = match change {
            TableChange::InsertRow { index, below, .. } | TableChange::CopyRow { index, below } => {
                let xml = index.unwrap_or(total.saturating_sub(1));
                let at = if below { xml + 1 } else { xml };
                let place = i64::try_from(rows_now.iter().position(|r| *r == at)?).ok()?;
                if place > last {
                    return Some(self.record.clone());
                }
                if place > top {
                    return None;
                }
                1
            }
            TableChange::DeleteRow { index } => {
                let Some(place) = self.accepted_index(index) else {
                    return Some(self.record.clone()); // a row already deleted as a tracked change
                };
                if place > last {
                    return Some(self.record.clone());
                }
                if place >= top {
                    return None;
                }
                -1
            }
            TableChange::InsertColumn { index, right } => {
                let column = self.accepted.from_grid(index);
                let at = i64::try_from(if right { column + 1 } else { column }).ok()?;
                let moved = columns(&self.record)
                    .into_iter()
                    .map(|c| shifted(c, |col| (col >= at).then_some(col + 1)))
                    .collect();
                return self.with_columns(moved);
            }
            TableChange::DeleteColumn { index } => {
                let at = i64::try_from(self.accepted.from_grid(index)).ok()?;
                let kept = columns(&self.record)
                    .into_iter()
                    .filter(|c| number(c, "col") != Some(at))
                    .map(|c| shifted(c, |col| (col > at).then_some(col - 1)))
                    .collect();
                return self.with_columns(kept);
            }
            TableChange::Merge { .. } | TableChange::Split { .. } => return None,
        };
        let mut record = self.record.clone();
        for key in ["header_row", "first_row"] {
            if let Some(n) = number(&record, key) {
                record.insert(key.to_owned(), Value::from(n + step));
            }
        }
        Some(record)
    }

    /// The record with `columns` in place of its own; `None` when none is left.
    fn with_columns(&self, columns: Vec<Map<String, Value>>) -> Option<Map<String, Value>> {
        if columns.is_empty() {
            return None;
        }
        let mut record = self.record.clone();
        record.insert(
            "columns".to_owned(),
            Value::Array(columns.into_iter().map(Value::Object).collect()),
        );
        Some(record)
    }

    /// The record `after` stored under the table's id now (`follow`): the entry under the
    /// id before dropped where it changed, the new one written where it differs.
    fn keep(&self, sink: &mut EditSink<'_, '_>, now: &str, after: Option<&Map<String, Value>>) -> Result<()> {
        if now != self.key {
            crate::provenance::store_record(sink, "table", &self.key, None)?;
        }
        if kind_of(now) == Some(Kind::Table) && (after != Some(&self.record) || now != self.key) {
            let text = after.map(|r| dumps(&Value::Object(r.clone())));
            crate::provenance::store_record(sink, "table", now, text.as_deref())?;
        }
        Ok(())
    }
}

/// The record's columns, each a map.
fn columns(record: &Map<String, Value>) -> Vec<Map<String, Value>> {
    record
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.as_object().cloned())
        .collect()
}

/// `column` with its `col` moved where `step` says.
fn shifted(mut column: Map<String, Value>, step: impl Fn(i64) -> Option<i64>) -> Map<String, Value> {
    if let Some(moved) = number(&column, "col").and_then(step) {
        column.insert("col".to_owned(), Value::from(moved));
    }
    column
}

/// The whole number at `key`, where there is one.
fn number(map: &Map<String, Value>, key: &str) -> Option<i64> {
    map.get(key).and_then(Value::as_i64)
}

/// How many rows a record's `rows` (`1-3,5`) names (`provenance.count`).
fn count(rows: &str) -> i64 {
    rows.split(',')
        .filter(|run| !run.is_empty())
        .map(|run| match run.split_once('-') {
            Some((first, last)) => {
                last.trim().parse::<i64>().unwrap_or(0) - first.trim().parse::<i64>().unwrap_or(0) + 1
            }
            None => 1,
        })
        .sum()
}

/// The rows of the table `target` names as accepted now.
fn rows_now(sink: &EditSink<'_, '_>, target: &str) -> Result<Vec<usize>> {
    let docx = sink.docx()?;
    let (story, table) = found_table(sink.doc(), &docx, target)?;
    Ok(TableGrid::new(story, &table).accepted(&table.id).rows)
}

impl Command<WordDoc> for KeepingRecord<EditTable> {
    type Out = TableChanged;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.0.validate_with(doc, Placing::recorded())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<TableChanged> {
        let mut sink = EditSink::new(tx, self.0.policy.clone());
        let before = Before::read(&mut sink, self.0.table.as_str())?;
        let out = self.0.edit_with(&mut sink, Placing::recorded())?;
        crate::after::after_edit(&mut sink)?;
        if let Some(before) = before {
            let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
            let after = before.after_change(self.0.change, &rows_now(&sink, &out.table)?);
            before.keep(&mut sink, &out.table, after.as_ref())?;
        }
        Ok(out)
    }
}

impl Command<WordDoc> for KeepingRecord<SetCells> {
    type Out = CellsSet;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.0.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<CellsSet> {
        let before = Before::read(&mut EditSink::new(tx, RevisionPolicy::Untracked), self.0.table.as_str())?;
        let out = self.0.prepare_placing(tx)?;
        if let Some(before) = before {
            let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
            let docx = sink.docx()?;
            let now = find_table(sink.doc(), &docx, self.0.table.as_str())?.id;
            let after = before.after_cells(&out.at);
            before.keep(&mut sink, &now, after.as_ref())?;
        }
        Ok(out)
    }
}
