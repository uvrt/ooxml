//! Owns: a table's properties recorded as Word records them when tracked (docx-agent's
//! `TableTracker`): a snapshot of the table's `w:tblPr`, grid, rows' `w:trPr` and cells'
//! `w:tcPr` before an edit, and after it -- once anything in the table changed -- the old
//! `w:tblPr` in a `w:tblPrChange` and the old grid in a `w:tblGridChange`, a row's old
//! `w:trPr` in a `w:trPrChange` where it or the grid changed, and every cell's old `w:tcPr`
//! in a `w:tcPrChange`, changed or not (Word's Reject All restores widths and spans only
//! from that whole record: measured).
//!
//! Must not: allocate revision ids but through its stamp, or record what an earlier record
//! of the same kind already holds.

use super::shape::{grid_widths, own_rows, row_cells};
use crate::names::W;
use crate::policy::Stamp;
use crate::review::xml::{insert_in_order, make};
use crate::sections::element_canonical as canonical;
use crate::validate::Orders;
use crate::work::{Id, Work};

/// A row's records, which a property edit leaves where they are.
const ROW_RECORDS: [&str; 3] = ["ins", "del", "trPrChange"];
/// A cell's records.
const CELL_RECORDS: [&str; 4] = ["cellIns", "cellDel", "cellMerge", "tcPrChange"];

/// A detached copy of `properties` (a new `w:{local}` where there are none) without the
/// children named `drop` (docx-agent's `clean`).
fn clean(work: &mut Work, properties: Option<Id>, local: &str, drop: &[&str]) -> Id {
    let out = make(work, local, &[]);
    if let Some(properties) = properties {
        for child in work.children(properties).to_vec() {
            if work.is_element(child) && !work.is_any(child, W, drop) {
                let copy = work.deep_copy(child);
                work.append(out, copy);
            }
        }
    }
    out
}

/// What a table was before an edit.
pub(crate) struct TableTracker {
    table: Id,
    properties: Id,
    grid: Vec<i64>,
    cells: Vec<(Id, Id)>,
    rows: Vec<(Id, Id)>,
}

impl TableTracker {
    /// The table as it is now; with `rows`, its rows' properties too.
    pub(crate) fn new(work: &mut Work, table: Id, rows: bool) -> TableTracker {
        let found = work.find(table, W, "tblPr");
        let properties = clean(work, found, "tblPr", &["tblPrChange"]);
        let grid = grid_widths(work, table);
        let own = own_rows(work, table);
        let mut cells = Vec::new();
        for row in &own {
            for placed in row_cells(work, *row) {
                let found = work.find(placed.cell, W, "tcPr");
                cells.push((placed.cell, clean(work, found, "tcPr", &CELL_RECORDS)));
            }
        }
        let mut snapshots = Vec::new();
        if rows {
            for row in own {
                let found = work.find(row, W, "trPr");
                snapshots.push((row, clean(work, found, "trPr", &ROW_RECORDS)));
            }
        }
        TableTracker {
            table,
            properties,
            grid,
            cells,
            rows: snapshots,
        }
    }

    /// Whether the cleaned `local` properties of `owner` differ from `old`.
    fn differs(work: &mut Work, owner: Id, local: &str, drop: &[&str], old: Id) -> bool {
        let found = work.find(owner, W, local);
        let now = clean(work, found, local, drop);
        canonical(work, now) != canonical(work, old)
    }

    /// Each cell watched, with its properties as they were.
    pub(crate) fn cells(&self) -> &[(Id, Id)] {
        &self.cells
    }

    /// Record what changed (`TableTracker.record`): the table's properties too with
    /// `table_level`, its grid with `grid`.
    pub(crate) fn record(&self, work: &mut Work, stamp: &mut Stamp, table_level: bool, grid: bool, orders: &Orders) {
        let table = self.table;
        let changed = Self::differs(work, table, "tblPr", &["tblPrChange"], self.properties);
        let grid_changed = grid_widths(work, table) != self.grid;
        let mut rows_changed = Vec::new();
        for (row, old) in &self.rows {
            if work.parent(*row).is_some() && Self::differs(work, *row, "trPr", &ROW_RECORDS, *old) {
                rows_changed.push(*row);
            }
        }
        let mut cells_changed = false;
        for (cell, old) in &self.cells {
            if work.parent(*cell).is_some() && Self::differs(work, *cell, "tcPr", &CELL_RECORDS, *old) {
                cells_changed = true;
                break;
            }
        }
        if !(changed || grid_changed || !rows_changed.is_empty() || cells_changed) {
            return;
        }
        let properties = work.find(table, W, "tblPr");
        if let Some(properties) = properties.filter(|p| table_level && work.find(*p, W, "tblPrChange").is_none()) {
            let record = stamp.make(work, "tblPrChange");
            let old = work.deep_copy(self.properties);
            work.append(record, old);
            insert_in_order(work, properties, record, orders);
        }
        let held = work.find(table, W, "tblGrid");
        if let Some(held) = held.filter(|g| grid && work.find(*g, W, "tblGridChange").is_none()) {
            let record = stamp.make(work, "tblGridChange");
            let old = make(work, "tblGrid", &[]);
            for width in &self.grid {
                let column = make(work, "gridCol", &[("w", &width.to_string())]);
                work.append(old, column);
            }
            work.append(record, old);
            insert_in_order(work, held, record, orders);
        }
        self.record_rows(work, stamp, grid_changed, &rows_changed, orders);
        self.record_cells(work, stamp, orders);
    }

    fn record_rows(&self, work: &mut Work, stamp: &mut Stamp, grid_changed: bool, changed: &[Id], orders: &Orders) {
        for row in own_rows(work, self.table) {
            let Some(&(_, old)) = self.rows.iter().find(|(r, _)| *r == row) else {
                continue;
            };
            if !(grid_changed || changed.contains(&row)) {
                continue;
            }
            let current = super::props::ensure(work, row, "trPr", orders);
            if work.find(current, W, "trPrChange").is_some() {
                continue;
            }
            let old = work.deep_copy(old);
            if grid_changed && work.find(old, W, "gridAfter").is_none() {
                let after = make(work, "gridAfter", &[("val", "0")]);
                insert_in_order(work, old, after, orders);
            }
            let record = stamp.make(work, "trPrChange");
            work.append(record, old);
            insert_in_order(work, current, record, orders);
        }
    }

    fn record_cells(&self, work: &mut Work, stamp: &mut Stamp, orders: &Orders) {
        for (cell, old) in &self.cells {
            if work.parent(*cell).is_none() {
                continue;
            }
            let current = match work.find(*cell, W, "tcPr") {
                Some(found) => found,
                None => {
                    let made = make(work, "tcPr", &[]);
                    work.insert(*cell, 0, made);
                    made
                }
            };
            if work.find(current, W, "tcPrChange").is_some() {
                continue;
            }
            let record = stamp.make(work, "tcPrChange");
            let old = work.deep_copy(*old);
            work.append(record, old);
            insert_in_order(work, current, record, orders);
        }
    }
}
