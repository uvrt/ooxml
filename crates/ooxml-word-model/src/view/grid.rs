//! Owns: a table's grid in a view, as Word rebuilds it from the cells the view leaves (Word
//! 16.106, lane fix-final-grid, `word-grid.json`; Accept All and Reject All alike):
//! - a grid column that only cells the view removes cover goes (docx2svg's ROADMAP
//!   "Tables -- measured" stage 7f);
//! - grid columns that no row of the view has a cell edge between are one column: a cell's
//!   span counts the view's columns, a column's width is theirs summed (`19830c99`,
//!   `HostSplit`; `RedundantNoChange`, the same with no pending change in the table, and
//!   `RedundantPlain`, Word's plain re-save of one with no revision at all);
//! - the grid spans to the widest row: a row past `w:tblGrid` adds columns (`Merge`, and the
//!   re-save of three corpus documents written so), and columns past every row go
//!   (`UncoveredColumn`); but a row reaching past the widest a table is fitted to
//!   ([`past_widest`]: 63 columns) widens nothing, Word re-griding it a way of its own, so the
//!   grid then stays `w:tblGrid` and a span past it counts its columns as written;
//! - each cell spans as its own side of a pending properties change (`w:tcPrChange`) has it,
//!   the original view's the old properties clean ([`View::cell_properties`], user decision
//!   original-clean-grid-2026-11-28), and a vertical merge either side has stays
//!   ([`View::merge_element`]).
//!
//! Positions here are the table's source grid columns (`w:tblGrid`), which ids and the
//! document's grid keep; a [`ViewGrid`] says which of them make each column of the view.
//!
//! Must not: build a model, word a change, or change a table.

use super::{content_kept, row_ends, View};
use crate::docx::tables::{grid_span, past_widest, row_cells, RowSpec};
use crate::xmlread::{attr_in, child_in, Ns};
use ooxml_xml::Element;
use std::collections::{BTreeSet, HashSet};
use std::ops::Range;

/// One row of a table as a view leaves it, in source grid columns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowShape {
    /// Where its first cell begins (`w:gridBefore`).
    pub before: usize,
    /// Its cells in order.
    pub cells: Vec<ShapeCell>,
    /// Where its `w:gridAfter` ends.
    pub end: usize,
}

/// A cell of a [`RowShape`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeCell {
    /// The source grid columns it covers.
    pub columns: Range<usize>,
    /// Whether the view keeps it there (`false`: a cell the view removes, one taken into the
    /// cell before it, or one gone with its end).
    pub kept: bool,
    /// A deleted cell whose content stays: its columns are the kept cell's before it, with
    /// no edge between them where they meet.
    pub widens: bool,
}

impl RowShape {
    /// `row` in `view`: `None` where the view removes the row.
    #[must_use]
    pub fn of(view: View, row: &Element) -> Option<RowShape> {
        if view.removes_row(row) {
            return None;
        }
        let spec = RowSpec::of(row);
        let all: Vec<_> = row_cells(row).into_iter().map(|(tc, _)| tc).collect();
        // the cells the final view takes in or loses with their ends, by place among those shown
        let shown: Vec<usize> = (0..all.len())
            .filter(|&k| all.get(k).is_some_and(|c| !view.removes_cell(c)))
            .collect();
        let lost: HashSet<usize> = if view == View::Final {
            row_ends(row)
                .gone
                .iter()
                .filter_map(|n| shown.get(*n).copied())
                .collect()
        } else {
            HashSet::new()
        };
        let mut out = RowShape {
            before: spec.before,
            ..RowShape::default()
        };
        let mut column = spec.before;
        for (k, cell) in all.iter().enumerate() {
            let columns = column..column.saturating_add(view.span(cell));
            column = columns.end;
            let entry = if view.removes_cell(cell) {
                let widens = view == View::Final && out.cells.iter().any(|c| c.kept) && content_kept(cell);
                ShapeCell {
                    columns,
                    kept: widens,
                    widens,
                }
            } else {
                ShapeCell {
                    columns,
                    kept: !lost.contains(&k),
                    widens: false,
                }
            };
            out.cells.push(entry);
        }
        out.end = column.saturating_add(spec.after);
        Some(out)
    }

    /// The places it has a cell edge at: each cell's start and end but where a cell widening
    /// the one before it meets it, its `w:gridBefore`'s end and its `w:gridAfter`'s.
    fn edges(&self) -> impl Iterator<Item = usize> + '_ {
        let joins = |k: usize| {
            matches!((k.checked_sub(1).and_then(|p| self.cells.get(p)), self.cells.get(k)),
                (Some(before), Some(cell)) if cell.widens && before.columns.end == cell.columns.start)
        };
        let inner = self.cells.iter().enumerate().flat_map(move |(k, c)| {
            let start = (!joins(k)).then_some(c.columns.start);
            let end = (!joins(k + 1)).then_some(c.columns.end);
            start.into_iter().chain(end)
        });
        [self.before, self.end].into_iter().chain(inner)
    }
}

/// A table's grid in a view: which source grid columns make each of its columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewGrid {
    /// Each column of the view, in order: the source grid columns it is made of.
    columns: Vec<Range<usize>>,
    /// The source grid's column count.
    count: usize,
    /// Where the view's grid ends, in source grid columns: the widest row's end, or the
    /// source grid's count for a table of no rows or one a row reaches past the widest a
    /// table is fitted to.
    reach: usize,
}

impl ViewGrid {
    /// The grid of a table of `count` source grid columns whose rows are `rows`, in `view`.
    #[must_use]
    pub fn of(view: View, rows: &[&Element], count: usize) -> ViewGrid {
        let shapes: Vec<RowShape> = rows.iter().filter_map(|r| RowShape::of(view, r)).collect();
        ViewGrid::build(&shapes, count)
    }

    /// The grid of a table of `count` source grid columns whose rows in the view are `rows`.
    #[must_use]
    pub fn build(rows: &[RowShape], count: usize) -> ViewGrid {
        let widest = rows.iter().map(|r| r.end).max().unwrap_or(count);
        let reach = if past_widest(count, widest) { count } else { widest };
        // the places a row of the view has a cell edge at, up to the widest row: the view's
        // columns run between them (by runs, so a span of billions of columns is one)
        let mut edges: BTreeSet<usize> = BTreeSet::from([0, reach]);
        for row in rows {
            edges.extend(row.edges());
        }
        let bounds: Vec<usize> = edges.into_iter().filter(|e| *e <= reach).collect();
        let runs: Vec<Range<usize>> = bounds
            .windows(2)
            .filter_map(|w| Some(*w.first()?..*w.get(1)?))
            .collect();
        // per run: whether every cell over it is one the view does not keep (None: no cell
        // over it, or a row's w:gridBefore or w:gridAfter)
        let mut only_removed: Vec<Option<bool>> = vec![None; runs.len()];
        let at = |column: usize| runs.partition_point(|r| r.end <= column);
        for row in rows {
            let mut mark = |columns: &Range<usize>, removed: bool| {
                for k in at(columns.start)..at(columns.end) {
                    if let Some(slot) = only_removed.get_mut(k) {
                        *slot = Some(slot.unwrap_or(true) && removed);
                    }
                }
            };
            mark(&(0..row.before), false);
            for cell in &row.cells {
                mark(&cell.columns, !cell.kept);
            }
            let last = row.cells.last().map_or(row.before, |c| c.columns.end);
            mark(&(last..row.end), false);
        }
        let columns = runs
            .into_iter()
            .zip(only_removed)
            .filter(|(_, removed)| *removed != Some(true))
            .map(|(run, _)| run)
            .collect();
        ViewGrid { columns, count, reach }
    }

    /// Each column of the view: the source grid columns it is made of.
    #[must_use]
    pub fn columns(&self) -> &[Range<usize>] {
        &self.columns
    }

    /// The view's column count.
    #[must_use]
    pub fn width(&self) -> usize {
        self.columns.len()
    }

    /// Where the widest row of the view ends, in source grid columns: the columns the view's
    /// grid is made of are below it.
    #[must_use]
    pub fn reach(&self) -> usize {
        self.reach
    }

    /// The source grid column each column of the view begins at.
    #[must_use]
    pub fn starts(&self) -> Vec<usize> {
        self.columns.iter().map(|c| c.start).collect()
    }

    /// Whether the view's grid is the source grid as it is.
    #[must_use]
    pub fn is_source(&self) -> bool {
        self.reach == self.count
            && self.columns.len() == self.count
            && self.columns.iter().enumerate().all(|(k, c)| *c == (k..k + 1))
    }

    /// The source grid's columns the view's grid keeps a place for: those before the widest
    /// row's end.
    #[must_use]
    pub fn grid_columns(&self) -> usize {
        self.count.min(self.reach)
    }

    /// How many of the view's columns begin past the source grid (the widest row's).
    #[must_use]
    pub fn past_grid(&self) -> usize {
        self.columns.iter().filter(|c| c.start >= self.count).count()
    }

    /// How many of the view's columns the source columns `columns` cover; past the view's
    /// grid (a row reaching past the widest a table is fitted to), each column counts.
    #[must_use]
    pub fn span(&self, columns: &Range<usize>) -> usize {
        let inside = self
            .columns
            .iter()
            .filter(|c| columns.start <= c.start && c.start < columns.end)
            .count();
        inside + columns.end.saturating_sub(columns.start.max(self.reach))
    }

    /// How many of the view's columns a cell over the source columns `parts` covers (its own,
    /// and those of the deleted cells whose content it takes).
    #[must_use]
    pub fn span_of(&self, parts: &[Range<usize>]) -> usize {
        parts.iter().map(|p| self.span(p)).sum()
    }

    /// The widths of the view's columns that begin inside the source grid, from its widths
    /// (`grid`, one per source column; a column past it has none of its own, the table's
    /// resolution sizes it as one written past the grid).
    #[must_use]
    pub fn widths(&self, grid: &[i64]) -> Vec<i64> {
        self.columns
            .iter()
            .filter(|c| c.start < self.count)
            .map(|c| {
                grid.get(c.start..c.end.min(self.count))
                    .unwrap_or_default()
                    .iter()
                    .sum()
            })
            .collect()
    }

    /// The source column a column of the view joined to the one before it begins it at:
    /// `Some(start)` for a source column inside a column of the view that begins earlier.
    #[must_use]
    pub fn joined_to(&self, column: usize) -> Option<usize> {
        self.columns
            .iter()
            .find(|c| c.start < column && column < c.end)
            .map(|c| c.start)
    }
}

impl View {
    /// The cell properties (`w:tcPr`) this view shows: in the original view the `w:tcPr` a
    /// `w:tcPrChange` holds -- the old properties, clean (user decision
    /// original-clean-grid-2026-11-28: Word's Reject All and its Review > Original also take
    /// the widths of the row's other cells away, drawing them 18 pt wide, which no view here
    /// follows); otherwise `properties` itself.
    #[must_use]
    pub fn cell_properties(self, properties: &Element) -> &Element {
        if self == View::Original {
            if let Some(old) = child_in(properties, Ns::W, "tcPrChange").and_then(|c| child_in(c, Ns::W, "tcPr")) {
                return old;
            }
        }
        properties
    }

    /// A cell's `w:gridSpan` in this view (at least 1), from [`View::cell_properties`].
    #[must_use]
    pub fn span(self, cell: &Element) -> usize {
        let raw = child_in(cell, Ns::W, "tcPr")
            .map(|p| self.cell_properties(p))
            .and_then(|p| child_in(p, Ns::W, "gridSpan"))
            .and_then(|g| attr_in(g, Some(Ns::W), "val"));
        grid_span(raw)
    }

    /// A cell's `w:vMerge` in this view: its own side's of a pending properties change
    /// ([`View::cell_properties`]), else the other side's -- Word keeps a vertical merge
    /// either side has (Accept All of a change that ends a merge keeps it, Reject All of one
    /// that makes it keeps it; Word 16.106, lane fix-final-grid, `VMerge`, `VMergeBack`).
    #[must_use]
    pub fn merge_element(self, cell: &Element) -> Option<&Element> {
        let properties = child_in(cell, Ns::W, "tcPr")?;
        let old = child_in(properties, Ns::W, "tcPrChange").and_then(|c| child_in(c, Ns::W, "tcPr"));
        let (own, other) = match self {
            View::Final => (Some(properties), old),
            View::Original => (old.or(Some(properties)), old.map(|_| properties)),
            View::Markup => (Some(properties), None),
        };
        [own, other]
            .into_iter()
            .flatten()
            .find_map(|p| child_in(p, Ns::W, "vMerge"))
    }
}

#[cfg(test)]
mod tests {
    use super::{RowShape, ShapeCell, ViewGrid};

    fn row(before: usize, cells: &[(usize, usize, bool)], end: usize) -> RowShape {
        RowShape {
            before,
            cells: cells
                .iter()
                .map(|&(s, e, kept)| ShapeCell {
                    columns: s..e,
                    kept,
                    widens: false,
                })
                .collect(),
            end,
        }
    }

    #[test]
    fn a_row_past_the_widest_table_widens_nothing() {
        let rows = [
            row(
                0,
                &[(0, 2_147_483_645, true), (2_147_483_645, 2_147_483_646, true)],
                2_147_483_646,
            ),
            row(0, &[(0, 1, true), (1, 2, true)], 2),
        ];
        let grid = ViewGrid::build(&rows, 2);
        assert!(grid.is_source());
        assert_eq!(grid.span(&(0..2_147_483_645)), 2_147_483_645);
    }

    #[test]
    fn a_widening_cell_joins_the_one_before_it_and_a_hole_between_still_goes() {
        let widening = |s: usize, e: usize| ShapeCell {
            columns: s..e,
            kept: true,
            widens: true,
        };
        let plain = |s: usize, e: usize, kept: bool| ShapeCell {
            columns: s..e,
            kept,
            widens: false,
        };
        // 19830c99: [D][deleted, content kept][E][F] beside [A span 2][B][C]
        let rows = [
            row(0, &[(0, 2, true), (2, 3, true), (3, 4, true)], 4),
            RowShape {
                before: 0,
                cells: vec![plain(0, 1, true), widening(1, 2), plain(2, 3, true), plain(3, 4, true)],
                end: 4,
            },
        ];
        assert_eq!(ViewGrid::build(&rows, 4).columns(), &[0..2, 2..3, 3..4]);
        // r55: a tracked merge's first cell, a deleted cell gone, two whose content stays,
        // above rows whose column 1 is deleted
        let rows = [
            RowShape {
                before: 0,
                cells: vec![plain(0, 1, true), plain(1, 2, false), widening(2, 3), widening(3, 4)],
                end: 4,
            },
            row(0, &[(0, 1, true), (1, 2, false), (2, 3, true), (3, 4, true)], 4),
        ];
        let grid = ViewGrid::build(&rows, 4);
        assert_eq!(grid.columns(), &[0..1, 2..3, 3..4]);
        assert_eq!(grid.span_of(&[0..1, 2..3, 3..4]), 3);
    }

    #[test]
    fn columns_no_row_divides_are_one() {
        // 19830c99 accepted: every row's first cell over grid columns 0 and 1
        let rows = [
            row(0, &[(0, 2, true), (2, 3, true), (3, 4, true)], 4),
            row(0, &[(0, 2, true), (2, 3, true), (3, 4, true)], 4),
        ];
        let grid = ViewGrid::build(&rows, 4);
        assert_eq!(grid.columns(), &[0..2, 2..3, 3..4]);
        assert_eq!(grid.span(&(0..2)), 1);
        assert_eq!(grid.widths(&[1512, 1512, 3024, 3024]), [3024, 3024, 3024]);
        assert_eq!(grid.joined_to(1), Some(0));
        assert_eq!(grid.joined_to(2), None);
        assert!(!grid.is_source());
    }

    #[test]
    fn one_row_dividing_keeps_the_columns() {
        let rows = [
            row(0, &[(0, 2, true), (2, 3, true)], 3),
            row(0, &[(0, 1, true), (1, 2, true), (2, 3, true)], 3),
        ];
        let grid = ViewGrid::build(&rows, 3);
        assert!(grid.is_source());
        assert_eq!(grid.span(&(0..2)), 2);
    }

    #[test]
    fn a_column_only_removed_cells_cover_goes() {
        let rows = [
            row(0, &[(0, 1, true), (1, 2, false), (2, 3, true)], 3),
            row(0, &[(0, 1, true), (1, 2, false), (2, 3, true)], 3),
        ];
        let grid = ViewGrid::build(&rows, 3);
        assert_eq!(grid.columns(), &[0..1, 2..3]);
        assert_eq!(grid.starts(), [0, 2]);
    }

    #[test]
    fn the_grid_spans_to_the_widest_row() {
        // Merge accepted: a row past the grid adds a column
        let rows = [
            row(0, &[(0, 3, true), (3, 4, true)], 4),
            row(0, &[(0, 1, true), (1, 2, true), (2, 3, true)], 3),
        ];
        let grid = ViewGrid::build(&rows, 3);
        assert_eq!(grid.columns(), &[0..1, 1..2, 2..3, 3..4]);
        assert_eq!(grid.span(&(0..3)), 3);
        assert_eq!(grid.widths(&[2000, 2000, 2000]), [2000, 2000, 2000]);
        assert_eq!(grid.reach(), 4);
        // UncoveredColumn: a column past every row goes
        let rows = [row(0, &[(0, 1, true), (1, 2, true), (2, 3, true)], 3)];
        let grid = ViewGrid::build(&rows, 4);
        assert_eq!(grid.columns(), &[0..1, 1..2, 2..3]);
        assert!(!grid.is_source());
    }
}
