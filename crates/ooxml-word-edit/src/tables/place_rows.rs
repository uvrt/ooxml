//! Owns: a source's rows placed into a table as one command ([`PlaceRows`]: docx-agent's
//! `_set_from`, `set_cells` `from`): rows added after the last, each as `insert_row` makes
//! one beside it with a header row's likeness made a body row (`_rows_after_last`, `grow`);
//! the block's cells written as `set_cells` writes them ([`crate::SetCells`]); and the
//! table's record of where its values came from kept under its durable id (`keep_provenance`)
//! -- in one transaction, one history step.
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
use ooxml_word_model::docx::provenance::{dumps, kind_of, Kind};
use serde_json::{Map, Value};
use std::time::Instant;

/// A block of a source's rows written into a table, rows added for it first.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceRows {
    /// The table (an id, an alias).
    pub table: String,
    /// Rows added after the last, each like it, before the cells are written.
    pub grow: usize,
    /// The cells, by document row and grid column (the rows added counted after the last),
    /// in order.
    pub writes: Vec<CellWrite>,
    /// The table's record of its source, replacing the one it has.
    pub record: Map<String, Value>,
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
    /// The table's id after the command.
    pub table: String,
    /// Where its time went, in seconds: `rows` (where it added any), `cells`, `record`.
    pub parts: Vec<(&'static str, f64)>,
}

impl Command<WordDoc> for PlaceRows {
    type Out = RowsPlaced;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        find_table(doc, &*doc.docx()?, &self.table).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<RowsPlaced> {
        let mut table = self.table.clone();
        let mut added = Vec::with_capacity(self.grow);
        let mut parts = Vec::new();
        let mut started = Instant::now();
        if self.grow > 0 {
            let mut sink = EditSink::new(tx, self.policy.clone());
            for _ in 0..self.grow {
                let change = EditTable {
                    table: table.clone(),
                    change: TableChange::InsertRow {
                        index: None,
                        below: true,
                        plain: true,
                    },
                    policy: self.policy.clone(),
                };
                let made = change.edit_placing(&mut sink)?;
                table.clone_from(&made.table);
                added.push(made);
            }
            crate::after::after_edit(&mut sink)?;
            parts.push(("rows", started.elapsed().as_secs_f64()));
            started = Instant::now();
        }
        let cells = SetCells {
            table: table.clone(),
            writes: self.writes.clone(),
            stamping: true,
            policy: self.policy.clone(),
            accepted: None,
        }
        .prepare_placing(tx)?;
        parts.push(("cells", started.elapsed().as_secs_f64()));
        let started = Instant::now();
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        let docx = sink.docx()?;
        let Found { id, .. } = find_table(sink.doc(), &docx, &table)?;
        if kind_of(&id) == Some(Kind::Table) {
            let text = dumps(&Value::Object(self.record.clone()));
            crate::provenance::store_record(&mut sink, "table", &id, Some(&text))?;
        }
        parts.push(("record", started.elapsed().as_secs_f64()));
        Ok(RowsPlaced {
            added,
            cells,
            table: id,
            parts,
        })
    }
}
