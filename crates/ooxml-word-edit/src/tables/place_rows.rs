//! Owns: a source's rows placed into a table as one command ([`PlaceRows`]: docx-agent's
//! `_set_from`, `set_cells` `from`, and `table_sync.sync`): rows added beside rows of the
//! table, each as `insert_row` makes one with a header row's likeness made a body row
//! (`_rows_after_last` for `grow`, `_rows_beside` for a sync); the block's cells written as
//! `set_cells` writes them ([`crate::SetCells`]); rows deleted (`delete_row`, a sync's
//! `remove`); and the table's record of where its values came from kept under its durable
//! id, or dropped (`keep_provenance`, `store_provenance`) -- in one transaction, one history
//! step.
//!
//! Must not: decide where a value goes or what the record says (the tool that placed the
//! block does), or keep the record of a table it did not write.

use super::rows::{EditTable, TableChange, TableChanged};
use super::{find_table, Found};
use crate::policy::RevisionPolicy;
use crate::set_cells::{CellWrite, CellsSet, SetCells};
use crate::sink::EditSink;
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::Address;
use ooxml_provenance::dumps;
use ooxml_word_model::docx::provenance::{kind_of, Kind};
use serde_json::{Map, Value};
use std::time::Instant;

/// Rows added beside one row of the table: the first beside it (below it unless not
/// `below`), each next one below the one before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowsBeside {
    /// The row, by its place in the table as it is when these are added (`None`: the last).
    pub row: Option<usize>,
    /// Below the row, else above it.
    pub below: bool,
    /// How many.
    pub count: usize,
}

/// What becomes of the table's record of its source.
#[derive(Debug, Clone, PartialEq)]
pub enum PlacedRecord {
    /// This record replaces the one the table has (under its durable id after the command;
    /// one kept under the id it had before is dropped).
    Store(Map<String, Value>),
    /// The record the table has is dropped (it no longer holds for every row).
    Drop,
    /// Left as it is (the table has none).
    Leave,
}

/// A block of a source's rows written into a table: rows added for it first, rows deleted
/// last.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceRows {
    /// The table (an id, an alias).
    pub table: ooxml_word_model::address::Address,
    /// The rows added, in order (from the bottom, so the rows above keep their places).
    pub beside: Vec<RowsBeside>,
    /// The cells, by document row and grid column as the rows added leave the table, in
    /// order.
    pub writes: Vec<CellWrite>,
    /// The rows deleted, by their places after the cells are written, in order (from the
    /// bottom).
    pub delete: Vec<usize>,
    /// What becomes of the table's record of its source.
    pub record: PlacedRecord,
    /// Untracked or tracked (the record is kept untracked: it is no content).
    pub policy: RevisionPolicy,
}

/// What a [`PlaceRows`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct RowsPlaced {
    /// The rows added, each as `insert_row` reports one, in order.
    pub added: Vec<TableChanged>,
    /// The cells written.
    pub cells: CellsSet,
    /// The rows deleted, each as `delete_row` reports one, in order.
    pub deleted: Vec<TableChanged>,
    /// The table's id after the command.
    pub table: String,
    /// Where its time went, in seconds: `rows` (where it added any), `cells`,
    /// `rows_removed` (where it deleted any), `record`.
    pub parts: Vec<(&'static str, f64)>,
}

impl PlaceRows {
    /// The table's rows changed through `sink`, one `change` per entry, each on the table as
    /// the one before left it; the table's id after.
    fn rows(
        &self,
        sink: &mut EditSink<'_, '_>,
        table: &mut Address,
        changes: impl IntoIterator<Item = TableChange>,
    ) -> Result<Vec<TableChanged>> {
        let mut out = Vec::new();
        for change in changes {
            let made = EditTable {
                table: table.clone(),
                change,
                policy: self.policy.clone(),
            }
            .edit_placing(sink)?;
            *table = Address::from(made.table.as_str());
            out.push(made);
        }
        Ok(out)
    }
}

/// The insertions [`RowsBeside`] names, in order.
fn insertions(beside: &[RowsBeside]) -> Vec<TableChange> {
    let mut out = Vec::new();
    for slot in beside {
        let (mut index, mut below) = (slot.row, slot.below);
        for _ in 0..slot.count {
            out.push(TableChange::InsertRow {
                index,
                below,
                plain: true,
            });
            index = index.map(|i| if below { i + 1 } else { i });
            below = true;
        }
    }
    out
}

impl Command<WordDoc> for PlaceRows {
    type Out = RowsPlaced;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        find_table(doc, &*doc.docx()?, self.table.as_str()).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<RowsPlaced> {
        let before = {
            let docx = tx.doc().docx()?;
            find_table(tx.doc(), &docx, self.table.as_str())?.id
        };
        let mut table = self.table.clone();
        let mut parts = Vec::new();
        let mut started = Instant::now();
        let mut added = Vec::new();
        if !self.beside.is_empty() {
            let mut sink = EditSink::new(tx, self.policy.clone());
            added = self.rows(&mut sink, &mut table, insertions(&self.beside))?;
            crate::after::after_edit(&mut sink)?;
            parts.push(("rows", started.elapsed().as_secs_f64()));
            started = Instant::now();
        }
        let cells = SetCells {
            table: table.clone(),
            writes: self.writes.clone(),
            policy: self.policy.clone(),
            accepted: None,
        }
        .prepare_placing(tx)?;
        parts.push(("cells", started.elapsed().as_secs_f64()));
        let mut deleted = Vec::new();
        if !self.delete.is_empty() {
            started = Instant::now();
            let mut sink = EditSink::new(tx, self.policy.clone());
            let changes = self.delete.iter().map(|index| TableChange::DeleteRow { index: *index });
            deleted = self.rows(&mut sink, &mut table, changes)?;
            crate::after::after_edit(&mut sink)?;
            parts.push(("rows_removed", started.elapsed().as_secs_f64()));
        }
        let started = Instant::now();
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        let docx = sink.docx()?;
        let Found { id, .. } = find_table(sink.doc(), &docx, table.as_str())?;
        let durable = |key: &str| kind_of(key) == Some(Kind::Table);
        match &self.record {
            PlacedRecord::Store(record) => {
                if before != id && durable(&before) {
                    crate::provenance::store_record(&mut sink, "table", &before, None)?;
                }
                if durable(&id) {
                    let text = dumps(&Value::Object(record.clone()));
                    crate::provenance::store_record(&mut sink, "table", &id, Some(&text))?;
                }
            }
            PlacedRecord::Drop if durable(&before) => {
                crate::provenance::store_record(&mut sink, "table", &before, None)?;
            }
            PlacedRecord::Drop | PlacedRecord::Leave => {}
        }
        parts.push(("record", started.elapsed().as_secs_f64()));
        Ok(RowsPlaced {
            added,
            cells,
            deleted,
            table: id,
            parts,
        })
    }
}
