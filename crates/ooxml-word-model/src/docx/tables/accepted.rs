//! Owns: a table as accepting its tracked changes leaves it, as docx-agent's tools count its
//! positions (`tools/objects.py`, `_Accepted`; `edit/tables.py`, `accepted_columns` and
//! `cells_lost_on_accept`; `edit/provenance.py`, `accepted_rows`): the document rows kept
//! (a row whose deletion is pending is not counted), the columns as accepted (Word's Accept
//! All rebuilds the grid, [`ViewGrid`]: a deleted cell merged into the cell beside it, or
//! gone with the cells after it moved left; grid columns no cell divides are one), the
//! positions a cell merged or taken into another covers, the cells Accept All removes with
//! their text; a position given as accepted mapped to the document's row and grid column --
//! row by row where the cells of a column as accepted are not at one grid column
//! ([`Accepted::to_row_grid`]), a whole column refused there as not supported
//! ([`Accepted::whole_columns`]) -- refused past the table (with its shape as accepted) or
//! in a cell Accept All removes, and what it stepped over or into said once as a note. The
//! one wording of these, for the tools and the edit core.
//!
//! Must not: change a table.

use super::labels::labelled;
use super::{grid_widths, placed_cells, TableGrid};
use crate::address::AddressError;
use crate::ids::W;
use crate::view::{cell_ends, View, ViewGrid};
use ooxml_base::pyfmt::repr_str;
use ooxml_xml::Element;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

/// Runs of grid columns of rows, each with a value: a cell's columns as one run, so a span
/// of billions of columns is one entry. The runs of one row do not overlap (a row's cells
/// do not).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnRuns<V> {
    /// `(row, first column)` to the run's end column and value.
    runs: BTreeMap<(usize, usize), (usize, V)>,
}

impl<V> Default for ColumnRuns<V> {
    fn default() -> Self {
        ColumnRuns { runs: BTreeMap::new() }
    }
}

impl<V: Copy> ColumnRuns<V> {
    /// `value` over the columns `columns` of row `row` (nothing for no columns).
    pub fn insert(&mut self, row: usize, columns: Range<usize>, value: V) {
        if !columns.is_empty() {
            self.runs.insert((row, columns.start), (columns.end, value));
        }
    }

    /// The value over grid position `(row, column)`, where a run covers it.
    #[must_use]
    pub fn get(&self, row: usize, column: usize) -> Option<V> {
        let (&(r, _), &(end, value)) = self.runs.range(..=(row, column)).next_back()?;
        (r == row && column < end).then_some(value)
    }

    /// Whether no run is there.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// Each run: its row, columns and value, by row then column.
    pub fn iter(&self) -> impl Iterator<Item = (usize, Range<usize>, V)> + '_ {
        self.runs.iter().map(|(&(r, start), &(end, v))| (r, start..end, v))
    }
}

/// The rows, columns and cells as accepted ([`Accepted`] without its texts).
#[derive(Debug, Default)]
pub(super) struct Layout {
    pub(super) kept: Vec<usize>,
    pub(super) given: ColumnRuns<usize>,
    pub(super) rows: Vec<usize>,
    pub(super) lost: BTreeSet<(usize, usize)>,
    pub(super) cell_ends: bool,
    pub(super) joined: Vec<Range<usize>>,
    pub(super) grid_columns: usize,
    pub(super) columns: usize,
    pub(super) deleted: Vec<usize>,
    pub(super) moved: Vec<Vec<usize>>,
    pub(super) view: Option<ViewGrid>,
}

/// A position refused: the error, and the argument it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Misplaced {
    /// `not_found` (past the table) or `refused` (a cell Accept All removes).
    pub error: AddressError,
    /// The argument, as the tool names it (`cells[0].col`).
    pub field: String,
}

/// A cell's row or column as a call gives it: a position, or a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis<'a> {
    /// A position, from 0, as accepted.
    At(i64),
    /// A row by its first cell's text, a column by its first row's.
    Label(&'a str),
}

/// A table as accepted (docx-agent's `_Accepted`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    /// The table's id.
    pub table: String,
    /// The grid columns kept, in order.
    pub kept: Vec<usize>,
    /// `(row, grid column)` of each position a cell taken into the one before it covers, to
    /// the grid column where that cell begins.
    pub given: ColumnRuns<usize>,
    /// The document rows kept, in order.
    pub rows: Vec<usize>,
    /// `(row, first grid column)` of each cell Accept All removes with its text.
    pub lost: BTreeSet<(usize, usize)>,
    /// Whether a cell's last paragraph mark is a pending deletion Accept All takes away (the
    /// next cell taken in, or the cell removed with its text).
    pub cell_ends: bool,
    /// The grid's column count as accepted where nothing is pending: `w:tblGrid`'s to the
    /// widest row, and one for each column of the view past it (positions count otherwise
    /// where the columns as accepted are not these).
    pub columns: usize,
    /// The document's row count.
    pub total_rows: usize,
    /// Each document row not kept: its first cell's text, deleted text included, shortened.
    row_texts: BTreeMap<usize, String>,
    /// Each grid column not kept: its first row's text, deleted text included.
    headers: BTreeMap<usize, String>,
    /// The positions of `given` whose cell is a pending tracked merge's (`w:cellDel`).
    merged: ColumnRuns<()>,
    /// The grid columns that are one column as accepted (no cell divides them), each run of
    /// two or more.
    joined: Vec<Range<usize>>,
    /// `w:tblGrid`'s columns before the widest row's end: those a column not kept is one of.
    grid_columns: usize,
    /// The grid columns accepting deletes: only cells it removes (or takes into another)
    /// cover them, in every row.
    deleted_columns: Vec<usize>,
    /// Per column as accepted, the document rows whose cell there is not at its own grid
    /// columns (cells before it removed and the cells after them moved left, or merged
    /// across it): such a column is no one grid column.
    moved: Vec<Vec<usize>>,
    /// The table's grid as accepted, row by row (`None` for a table with nothing pending).
    view: Option<ViewGrid>,
}

/// The form refused where a column position of a table as accepted is no one grid column
/// (`details.not_supported`).
pub const MOVED_COLUMN: &str = "a column position whose cells pending cell changes move";

/// The grid column the first row's cell over column `column` of `grid` begins at (a column
/// of the view at no grid line), or the last grid line before it.
fn first_cell_at(grid: &ViewGrid, column: usize) -> usize {
    grid.rows()
        .iter()
        .flatten()
        .find_map(|r| r.at(column).map(|c| c.source.start))
        .or_else(|| (0..column).rev().find_map(|k| grid.source_line(k)))
        .unwrap_or(0)
}

/// Python's `" ".join(text.split())`, shortened to `limit` characters with an ellipsis
/// (docx-agent's `short`).
fn short(text: &str, limit: usize) -> String {
    let text = super::collapse_space(text);
    if text.chars().count() <= limit {
        return text;
    }
    let mut out: String = text.chars().take(limit.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// An element's `w:t` and `w:delText` text, in document order.
fn all_text(element: &Element, out: &mut String) {
    for child in element.child_elements() {
        if child.name().is(Some(W), "t") || child.name().is(Some(W), "delText") {
            out.push_str(&crate::docx::text::leading_text(child));
        }
        all_text(child, out);
    }
}

fn joined(items: &[usize]) -> String {
    items.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
}

/// `n (repr)` for each of `items` with a text, else `n` (`f"{r} ({n!r})" if n else str(r)`).
fn said(items: &[usize], texts: &BTreeMap<usize, String>) -> String {
    items
        .iter()
        .map(|k| match texts.get(k).filter(|t| !t.is_empty()) {
            Some(text) => format!("{k} ({})", repr_str(text)),
            None => k.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Positions `0..n` as text, at most 50 (a refusal's options).
fn positions(n: usize) -> Vec<String> {
    (0..n.min(50)).map(|k| k.to_string()).collect()
}

/// `n - 1` as Python prints it (`-1` for none).
fn last(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX) - 1
}

impl TableGrid<'_> {
    /// The table as accepting its tracked changes leaves it; `id` its id.
    #[must_use]
    pub fn accepted(&self, id: &str) -> Accepted {
        let Layout {
            kept,
            given,
            rows,
            lost,
            cell_ends,
            joined,
            grid_columns,
            columns,
            deleted,
            moved,
            view,
        } = self.accepted_layout();
        let row_texts = (0..self.rows.len())
            .filter(|r| !rows.contains(r))
            .map(|r| (r, self.first_cell_text(r)))
            .collect();
        let headers = deleted
            .iter()
            .map(|&c| (c, self.cell_all_text(0, c).unwrap_or_default()))
            .collect();
        // a run is one cell's columns, so one position of it answers for all
        let mut merged = ColumnRuns::default();
        for (r, columns, _) in given.iter() {
            if self
                .cell_element(r, columns.start)
                .is_none_or(|e| View::Final.removes_cell(e))
            {
                merged.insert(r, columns, ());
            }
        }
        Accepted {
            table: id.to_owned(),
            kept,
            given,
            rows,
            lost,
            cell_ends,
            columns,
            total_rows: self.rows.len(),
            row_texts,
            headers,
            merged,
            joined,
            grid_columns,
            deleted_columns: deleted,
            moved,
            view,
        }
    }

    /// The rows, columns and cells as accepted, without the texts a note says.
    pub(super) fn accepted_layout(&self) -> Layout {
        let mut out = Layout::default();
        let columns = self.column_count();
        for (r, row) in self.rows.iter().enumerate() {
            let deleted_row = View::Final.removes_row(row);
            if !deleted_row {
                out.rows.push(r);
            }
            let cells = placed_cells(row);
            let shown: Vec<&(Arc<Element>, usize, usize)> =
                cells.iter().filter(|c| !View::Final.removes_cell(&c.0)).collect();
            let elements: Vec<&Element> = shown.iter().map(|c| &*c.0).collect();
            let ends = cell_ends(&elements);
            // a row whose deletion is pending goes whole: its cells' ends take nothing in
            out.cell_ends |= !deleted_row && ends.any();
            if !deleted_row {
                for n in ends.lost() {
                    if let Some(cell) = shown.get(n) {
                        out.lost.insert((r, cell.1));
                    }
                }
            }
        }
        let rows: Vec<&Element> = self.rows.iter().map(|r| &**r).collect();
        let grid = ViewGrid::of(View::Final, &rows, &grid_widths(&self.table));
        // a cell merged into one beside it, or taken in, answers as that cell
        for (r, row) in grid.rows().iter().enumerate() {
            let (Some(row), Some(cells)) = (row, self.rows.get(r).map(|e| placed_cells(e))) else {
                continue;
            };
            for cell in &row.cells {
                for part in cell.parts.iter().filter(|p| **p != cell.cell) {
                    if let Some((_, start, span)) = cells.get(*part) {
                        out.given
                            .insert(r, *start..start.saturating_add(*span), cell.source.start);
                    }
                }
            }
        }
        let reach = grid.reach();
        out.grid_columns = columns.min(reach);
        out.columns = out.grid_columns + reach.saturating_sub(columns);
        let width = grid.width();
        out.kept = (0..width)
            .map(|k| {
                grid.source_column(k)
                    .map(|c| c.start)
                    .or_else(|| grid.source_line(k))
                    .unwrap_or_else(|| first_cell_at(&grid, k))
            })
            .collect();
        out.joined = (0..width)
            .filter_map(|k| grid.source_column(k))
            .filter(|c| c.len() > 1)
            .collect();
        out.moved = (0..width).map(|k| grid.moved_rows(k)).collect();
        // a grid column only cells accepting removes with nothing kept cover, in every row
        // (one merged into a cell beside it, or taken in, is that cell's)
        let covered: Vec<Vec<Range<usize>>> = grid
            .rows()
            .iter()
            .enumerate()
            .filter_map(|(r, row)| {
                let row = row.as_ref()?;
                let cells = self.rows.get(r).map(|e| placed_cells(e)).unwrap_or_default();
                let mut out: Vec<Range<usize>> = vec![row.source_before.clone(), row.source_after.clone()];
                for cell in &row.cells {
                    out.extend(
                        cell.parts
                            .iter()
                            .filter_map(|p| cells.get(*p).map(|(_, s, n)| *s..s.saturating_add(*n))),
                    );
                }
                Some(out)
            })
            .collect();
        out.deleted = (0..out.grid_columns)
            .filter(|c| covered.iter().all(|row| !row.iter().any(|r| r.contains(c))))
            .collect();
        out.view = Some(grid);
        out
    }

    /// The `w:tc` over grid position `(row, column)`.
    fn cell_element(&self, row: usize, column: usize) -> Option<&Element> {
        let origin = self.origin(row, column)?;
        self.cells.get(&origin).map(|(e, _)| &**e)
    }

    /// Row `row`'s first `w:tc` (a child of the row) text, deleted text included, shortened
    /// to 40 characters (docx-agent's `_row_text`).
    fn first_cell_text(&self, row: usize) -> String {
        let Some(cell) = self.rows.get(row).and_then(|r| r.find(Some(W), "tc")) else {
            return String::new();
        };
        let mut text = String::new();
        all_text(cell, &mut text);
        short(&text, 40)
    }

    /// The text, deleted text included, of the cell over `(row, column)`, white space
    /// collapsed (docx-agent's `_deleted_header`).
    fn cell_all_text(&self, row: usize, column: usize) -> Option<String> {
        let mut text = String::new();
        all_text(self.cell_element(row, column)?, &mut text);
        Some(super::collapse_space(&text))
    }

    /// The document row whose first cell (`column`, a grid column) has the text `label`,
    /// among `rows` (`_locate`'s row label).
    ///
    /// # Errors
    /// `not_found` or `ambiguous`, the texts the options.
    pub fn row_labelled_among(&self, rows: &[usize], column: usize, label: &str) -> Result<usize, AddressError> {
        let texts: Vec<String> = rows
            .iter()
            .map(|r| self.cell_text(*r, column).unwrap_or_default())
            .collect();
        let hit = labelled(&texts, label, "row")?;
        Ok(rows.get(hit).copied().unwrap_or(hit))
    }

    /// The grid column among `columns` whose cell in `header_row` has the text `label`.
    ///
    /// # Errors
    /// `not_found` or `ambiguous`, the texts the options.
    pub fn column_labelled_among(
        &self,
        columns: &[usize],
        header_row: usize,
        label: &str,
    ) -> Result<usize, AddressError> {
        let texts: Vec<String> = columns
            .iter()
            .map(|c| self.cell_text(header_row, *c).unwrap_or_default())
            .collect();
        let hit = labelled(&texts, label, "column")?;
        Ok(columns.get(hit).copied().unwrap_or(hit))
    }
}

impl Accepted {
    /// A table `table` of `columns` grid columns and `rows` rows with nothing pending.
    #[must_use]
    pub fn plain(table: &str, columns: usize, rows: usize) -> Accepted {
        Accepted {
            table: table.to_owned(),
            kept: (0..columns).collect(),
            given: ColumnRuns::default(),
            rows: (0..rows).collect(),
            lost: BTreeSet::new(),
            cell_ends: false,
            columns,
            total_rows: rows,
            row_texts: BTreeMap::new(),
            headers: BTreeMap::new(),
            merged: ColumnRuns::default(),
            joined: Vec::new(),
            grid_columns: columns,
            deleted_columns: Vec::new(),
            moved: Vec::new(),
            view: None,
        }
    }

    /// Whether the columns or cells count otherwise as accepted.
    #[must_use]
    pub fn active(&self) -> bool {
        self.kept.len() != self.columns || !self.given.is_empty() || !self.lost.is_empty() || self.any_moved()
    }

    /// Whether a column as accepted is no one grid column: a row's cell there moved (cells
    /// before it removed) or merged across it.
    #[must_use]
    pub fn any_moved(&self) -> bool {
        self.moved.iter().any(|rows| !rows.is_empty())
    }

    /// Refused where column positions `columns` as accepted are not each one grid column in
    /// every row: Word's Accept All removes cells and moves the cells after them left before
    /// it rebuilds the grid, so such a column is a different grid column in different rows
    /// (`field` the argument).
    ///
    /// # Errors
    /// `not_supported` at `field`, naming the rows (as accepted) where it differs.
    pub fn whole_columns(&self, columns: Range<usize>, field: &str) -> Result<(), Misplaced> {
        let mut rows: Vec<usize> = columns
            .clone()
            .filter_map(|c| self.moved.get(c))
            .flatten()
            .map(|r| self.from_row(*r))
            .collect();
        if rows.is_empty() {
            return Ok(());
        }
        rows.sort_unstable();
        rows.dedup();
        let which = if columns.len() == 1 {
            format!("col {}", columns.start)
        } else {
            format!("cols {} to {}", columns.start, columns.end - 1)
        };
        let message = format!(
            "{field}: {which} of {} as accepted is not one grid column in every row: in row(s) {} pending cell \
             changes remove cells, and Word's Accept All moves the cells after them left before it rebuilds the grid. \
             Accept or reject the pending cell changes first, or edit the cells by row",
            self.table,
            joined(&rows)
        );
        Err(Misplaced {
            error: AddressError {
                code: "not_supported",
                message,
                options: Vec::new(),
            },
            field: field.to_owned(),
        })
    }

    /// Whether the rows count otherwise as accepted.
    #[must_use]
    pub fn rows_active(&self) -> bool {
        self.rows.len() != self.total_rows
    }

    /// Rows as accepted.
    #[must_use]
    pub fn height(&self) -> usize {
        self.rows.len()
    }

    /// Columns as accepted.
    #[must_use]
    pub fn width(&self) -> usize {
        self.kept.len()
    }

    /// What its rows are as accepted (`row_shape`).
    #[must_use]
    pub fn row_shape(&self) -> String {
        let skipped: Vec<usize> = (0..self.total_rows).filter(|r| !self.rows.contains(r)).collect();
        let mut out = format!("{} has rows 0 to {}", self.table, last(self.height()));
        if !skipped.is_empty() {
            out.push_str(&format!(
                " as accepted (document row(s) {} deleted by a pending tracked change, not counted)",
                joined(&skipped)
            ));
        }
        out
    }

    /// What its columns are as accepted (`shape`).
    #[must_use]
    pub fn shape(&self) -> String {
        let skipped = self.deleted_columns(self.grid_columns);
        let mut out = format!("{} has columns 0 to {}", self.table, last(self.width()));
        if !skipped.is_empty() {
            out.push_str(&format!(
                " as accepted (grid column(s) {} deleted by a pending tracked change, not counted)",
                joined(&skipped)
            ));
        }
        if let Some(said) = self.joining(usize::MAX) {
            out.push_str(if skipped.is_empty() { " as accepted; " } else { "; " });
            out.push_str(&said);
        }
        out
    }

    /// The grid columns accepting deletes: not kept, and not joined to a column before them
    /// (docx-agent's `columns_deleted_pending`).
    #[must_use]
    pub fn deleted(&self) -> Vec<usize> {
        self.deleted_columns(self.grid_columns)
    }

    /// The grid columns before `end` accepting deletes (not kept, not joined to a column
    /// before them).
    fn deleted_columns(&self, end: usize) -> Vec<usize> {
        self.deleted_columns.iter().copied().filter(|c| *c < end).collect()
    }

    /// The runs of grid columns before `end` that are one column as accepted, said: `grid
    /// columns 0 and 1 are one column in this view (no cell divides them)`.
    fn joining(&self, end: usize) -> Option<String> {
        let runs: Vec<String> = self
            .joined
            .iter()
            .filter(|j| j.start < end)
            .map(|j| match j.len() {
                2 => format!("{} and {}", j.start, j.start + 1),
                _ => format!("{} to {}", j.start, j.end - 1),
            })
            .collect();
        (!runs.is_empty()).then(|| {
            let verb = if runs.len() == 1 { "are" } else { "are each" };
            format!(
                "grid columns {} {verb} one column in this view (no cell divides them)",
                runs.join(", ")
            )
        })
    }

    /// The document row of position `row`, refused past the last; a row skipped said once
    /// in `notes` (`to_row`).
    ///
    /// # Errors
    /// `not_found` at `field`, the positions the options.
    pub fn to_row(&self, row: i64, field: &str, notes: &mut Vec<String>) -> Result<usize, Misplaced> {
        let Some(found) = usize::try_from(row).ok().and_then(|r| self.rows.get(r).copied()) else {
            return Err(Misplaced {
                error: AddressError::not_found(
                    format!("{field} {row}: {}", self.row_shape()),
                    positions(self.height()),
                ),
                field: field.to_owned(),
            });
        };
        if usize::try_from(row).ok() != Some(found) {
            let gone: Vec<usize> = (0..found).filter(|r| !self.rows.contains(r)).collect();
            let note = format!(
                "row positions count {} as accepted: document row {} has a tracked deletion not yet accepted and is \
                 not counted (word_changes reaches it; cell ids keep the document's rows)",
                self.table,
                said(&gone, &self.row_texts)
            );
            if !notes.contains(&note) {
                notes.push(note);
            }
        }
        Ok(found)
    }

    /// The position of document row `row` (of the kept row before it, for one not kept).
    #[must_use]
    pub fn from_row(&self, row: usize) -> usize {
        self.rows.iter().filter(|r| **r <= row).count().saturating_sub(1)
    }

    /// The grid column of position `column`, refused past the last; a column skipped said
    /// once in `notes` (`to_grid`).
    ///
    /// # Errors
    /// `not_found` at `field`, the positions the options.
    pub fn to_grid(&self, column: i64, field: &str, notes: &mut Vec<String>) -> Result<usize, Misplaced> {
        let Some(grid) = usize::try_from(column).ok().and_then(|c| self.kept.get(c).copied()) else {
            return Err(self.past_columns(column, field));
        };
        if let Ok(at) = usize::try_from(column) {
            self.whole_columns(at..at + 1, field)?;
        }
        if usize::try_from(column).ok() != Some(grid) {
            let gone = self.deleted_columns(grid);
            let head = if gone.is_empty() {
                format!("col positions count {} as accepted", self.table)
            } else {
                format!(
                    "col positions count {} as accepted: grid column {} has a tracked deletion not yet accepted and is \
                     not counted (word_changes reaches it)",
                    self.table,
                    said(&gone, &self.headers)
                )
            };
            let head = match self.joining(grid) {
                Some(joining) if gone.is_empty() => format!("{head}: {joining}"),
                Some(joining) => format!("{head}; {joining}"),
                None => head,
            };
            if !notes.iter().any(|n| n.starts_with(&head)) {
                notes.push(format!("{head}; col {column} is grid column {grid}"));
            }
        }
        Ok(grid)
    }

    fn past_columns(&self, column: i64, field: &str) -> Misplaced {
        Misplaced {
            error: AddressError::not_found(format!("{field} {column}: {}", self.shape()), positions(self.width())),
            field: field.to_owned(),
        }
    }

    /// The grid column of position `column` in document row `row`: the cell there as
    /// accepted (a column moved in some rows is that row's own cell), refused past the
    /// last; what it stepped over said once in `notes`.
    ///
    /// # Errors
    /// `not_found` at `field`, the positions the options.
    pub fn to_row_grid(
        &self,
        row: usize,
        column: i64,
        field: &str,
        notes: &mut Vec<String>,
    ) -> Result<usize, Misplaced> {
        let at = usize::try_from(column).ok().filter(|c| *c < self.width());
        let moved = at.and_then(|c| self.moved.get(c)).is_some_and(|rows| !rows.is_empty());
        let (Some(at), true) = (at, moved) else {
            return self.to_grid(column, field, notes);
        };
        let laid = self.view.as_ref().and_then(|v| v.row(row));
        let Some(cell) = laid.and_then(|r| r.at(at)) else {
            return Err(self.past_columns(column, field));
        };
        let view = self.view.as_ref().and_then(|v| Some((v, v.row(row)?)));
        let mapped = view.and_then(|(v, r)| v.row_source(r, at)).flatten();
        let grid = mapped.unwrap_or(cell.source.start);
        if !cell.in_place && usize::try_from(column).ok() != Some(grid) {
            let note = format!(
                "col positions count row {} of {} as accepted: pending cell changes remove cells before it, and the \
                 cells after them move left; col {column} is grid column {grid} in that row",
                self.from_row(row),
                self.table
            );
            if !notes.contains(&note) {
                notes.push(note);
            }
        }
        Ok(grid)
    }

    /// The position of grid column `grid` (of the kept column before it, for one not kept).
    #[must_use]
    pub fn from_grid(&self, grid: usize) -> usize {
        self.kept.iter().filter(|c| **c <= grid).count().saturating_sub(1)
    }

    /// Where `(row, grid)` (document row, grid column) is written: a cell taken into the
    /// one before it answers as that cell, said once in `notes`; one Accept All removes with
    /// its text is refused, with the reject that keeps it (`at`).
    ///
    /// # Errors
    /// `refused` at `cells`.
    pub fn at(&self, row: usize, grid: usize, notes: &mut Vec<String>) -> Result<usize, Misplaced> {
        let (shown_row, shown_grid) = (self.from_row(row), self.from_grid(grid));
        if self.lost.contains(&(row, grid)) {
            let message = format!(
                "({shown_row}, {shown_grid}) of {} is a cell accepting removes (its last paragraph mark is deleted \
                 and no cell follows it in its row): Word's Accept All takes its text out of view, word_changes \
                 accept moves the text after the row. Reject that change first to write in the cell (check's \
                 accept_loses_text gives the call)",
                self.table
            );
            return Err(Misplaced {
                error: AddressError {
                    code: "refused",
                    message,
                    options: vec!["word_changes reject".to_owned()],
                },
                field: "cells".to_owned(),
            });
        }
        let Some(found) = self.given.get(row, grid) else {
            return Ok(grid);
        };
        let how = if self.merged.get(row, grid).is_some() {
            "a pending tracked merge"
        } else {
            "accepting a deleted paragraph mark"
        };
        let note = format!(
            "({shown_row}, {shown_grid}) is in the cell {how} makes of ({shown_row}, {}): written there",
            self.from_grid(found)
        );
        if !notes.contains(&note) {
            notes.push(note);
        }
        Ok(found)
    }

    /// Position `(row, column)` as (document row, grid column), as accepted (`place`):
    /// `field` the argument (`first`), its `.row` and `.col` refused.
    ///
    /// # Errors
    /// As [`Accepted::to_row`], [`Accepted::to_grid`] and [`Accepted::at`].
    pub fn place(
        &self,
        row: i64,
        column: i64,
        field: &str,
        notes: &mut Vec<String>,
    ) -> Result<(usize, usize), Misplaced> {
        let as_index = |n: i64| usize::try_from(n).unwrap_or(usize::MAX);
        let row = if self.rows_active() {
            self.to_row(row, &format!("{field}.row"), notes)?
        } else {
            as_index(row)
        };
        if !self.active() {
            return Ok((row, as_index(column)));
        }
        let grid = self.to_row_grid(row, column, &format!("{field}.col"), notes)?;
        Ok((row, self.at(row, grid, notes)?))
    }

    /// A cell's (document row, grid column) by position or labels (`_locate` with
    /// `accepted`), its labels read on `grid` (the table as the call's earlier writes left
    /// it): a row label among the rows as accepted, in the first column kept; a column label
    /// among the columns kept, in the first row.
    ///
    /// # Errors
    /// As [`Accepted::place`]; a label matching no row or column, or several, at no field.
    pub fn locate(
        &self,
        grid: &TableGrid<'_>,
        (row, column): (Axis<'_>, Axis<'_>),
        notes: &mut Vec<String>,
    ) -> Result<(usize, usize), Misplaced> {
        let unplaced = |error: AddressError| Misplaced {
            error,
            field: String::new(),
        };
        let all: Vec<usize>;
        let rows: &[usize] = if self.rows_active() {
            &self.rows
        } else {
            all = (0..grid.row_count()).collect();
            &all
        };
        let first = match self.kept.first() {
            Some(first) if self.active() => *first,
            _ => 0,
        };
        let row = match row {
            Axis::At(at) if self.rows_active() => self.to_row(at, "cells[].row", notes)?,
            Axis::At(at) => usize::try_from(at).unwrap_or(usize::MAX),
            Axis::Label(label) => grid.row_labelled_among(rows, first, label).map_err(unplaced)?,
        };
        if !self.active() {
            let column = match column {
                Axis::At(at) => usize::try_from(at).unwrap_or(usize::MAX),
                Axis::Label(label) => grid.column_labelled(0, label).map_err(unplaced)?,
            };
            return Ok((row, column));
        }
        let grid_column = match column {
            Axis::At(at) => self.to_row_grid(row, at, "cells[].col", notes)?,
            Axis::Label(label) => {
                let found = grid.column_labelled_among(&self.kept, 0, label).map_err(unplaced)?;
                let at = self.from_grid(found);
                self.whole_columns(at..at + 1, "cells[].col")?;
                found
            }
        };
        Ok((row, self.at(row, grid_column, notes)?))
    }
}
