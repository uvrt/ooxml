//! Owns: [`GridPlaces`], each grid position's cell as docx-agent's `Table.grid` gives it
//! (the cell's origin as its first place `(row, column)`, nothing where no cell covers the
//! position), kept as one run of columns per cell, so its size is the table's cells, not its
//! grid columns: a `w:gridSpan` or `w:gridBefore` of two billion columns is one run.
//!
//! Must not: read elements (it is made from a [`Grid`]) or decide what a view keeps.

use super::grid::Grid;
use std::ops::Range;

/// A grid position, `(row, column)`.
pub type Spot = (usize, usize);

/// One cell's grid columns in a row, and the first place of its origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceRun {
    /// The grid columns it covers.
    pub columns: Range<usize>,
    /// The first place `(row, column)` of the cell its vertical merge starts at.
    pub origin: Spot,
}

/// Each grid position's cell, by rows, as runs of columns in column order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GridPlaces {
    rows: Vec<Vec<PlaceRun>>,
    /// Each row's grid column after its last cell (its `w:gridBefore` where it has none).
    ends: Vec<usize>,
}

impl GridPlaces {
    /// The places of `grid`: each cell over its columns, a merged cell's origin as its first
    /// place.
    #[must_use]
    pub fn of(grid: &Grid) -> GridPlaces {
        let rows = grid
            .rows
            .iter()
            .map(|line| {
                line.cells
                    .iter()
                    .map(|cell| {
                        let (r, k) = cell.origin;
                        let first = grid
                            .rows
                            .get(r)
                            .and_then(|o| o.cells.get(k))
                            .map_or(cell.first, |o| o.first);
                        PlaceRun {
                            columns: cell.first..cell.first.saturating_add(cell.span),
                            origin: (r, first),
                        }
                    })
                    .collect()
            })
            .collect();
        let ends = grid.rows.iter().map(|line| line.end).collect();
        GridPlaces { rows, ends }
    }

    /// The number of rows.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The grid columns the longest row reaches: its last cell's end (`w:gridAfter` not
    /// counted).
    #[must_use]
    pub fn width(&self) -> usize {
        self.ends.iter().copied().max().unwrap_or(0)
    }

    /// The grid column after row `row`'s last cell, its `w:gridBefore` where it has none (0
    /// past the last row).
    #[must_use]
    pub fn end(&self, row: usize) -> usize {
        self.ends.get(row).copied().unwrap_or(0)
    }

    /// The runs of row `row`, in column order (none past the last row).
    #[must_use]
    pub fn runs(&self, row: usize) -> &[PlaceRun] {
        self.rows.get(row).map_or(&[], Vec::as_slice)
    }

    /// How many rows and grid columns the positions whose cell is `origin` take: the rows
    /// holding one, and the columns any of them covers (counted by runs).
    #[must_use]
    pub fn size_of(&self, origin: Spot) -> (usize, usize) {
        let mut rows = 0;
        let mut columns: Vec<Range<usize>> = Vec::new();
        for runs in &self.rows {
            let mut held = false;
            for run in runs.iter().filter(|run| run.origin == origin) {
                held = true;
                columns.push(run.columns.clone());
            }
            rows += usize::from(held);
        }
        columns.sort_by_key(|c| c.start);
        let mut count = 0usize;
        let mut reached = 0usize;
        for c in columns {
            let start = c.start.max(reached);
            if c.end > start {
                count = count.saturating_add(c.end - start);
                reached = c.end;
            }
        }
        (rows, count)
    }

    /// The first place of the cell over grid position `(row, column)`, where one covers it.
    #[must_use]
    pub fn get(&self, row: usize, column: usize) -> Option<Spot> {
        let runs = self.rows.get(row)?;
        let k = runs.partition_point(|run| run.columns.end <= column);
        runs.get(k)
            .filter(|run| run.columns.contains(&column))
            .map(|run| run.origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docx::tables::{CellSpec, RowSpec, VMerge, GRID_VALUE_MAX};

    fn row(before: &str, cells: &[(&str, VMerge)]) -> RowSpec {
        RowSpec::new(
            Some(before),
            None,
            cells
                .iter()
                .map(|(span, merge)| CellSpec::new(Some(span), *merge))
                .collect(),
        )
    }

    #[test]
    fn places_answer_as_the_dense_grid_did() {
        let specs = [
            row("1", &[("2", VMerge::Restart), ("1", VMerge::None)]),
            row("0", &[("1", VMerge::None), ("2", VMerge::Continue)]),
        ];
        let places = GridPlaces::of(&Grid::build(&specs, 4));
        let dense: Vec<Vec<Option<Spot>>> = (0..places.row_count())
            .map(|r| (0..5).map(|c| places.get(r, c)).collect())
            .collect();
        assert_eq!(
            dense,
            vec![
                vec![None, Some((0, 1)), Some((0, 1)), Some((0, 3)), None],
                vec![Some((1, 0)), Some((0, 1)), Some((0, 1)), None, None],
            ]
        );
        assert_eq!(places.width(), 4);
        assert_eq!(places.get(2, 0), None);
        for origin in [(0, 1), (0, 3), (1, 0), (9, 9)] {
            let held: Vec<(usize, usize)> = (0..2)
                .flat_map(|r| (0..5).map(move |c| (r, c)))
                .filter(|&(r, c)| places.get(r, c) == Some(origin))
                .collect();
            let rows = held
                .iter()
                .map(|p| p.0)
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            let columns = held
                .iter()
                .map(|p| p.1)
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            assert_eq!(places.size_of(origin), (rows, columns), "{origin:?}");
        }
        let only_before = GridPlaces::of(&Grid::build(&[row("3", &[])], 0));
        assert_eq!(
            (only_before.width(), only_before.end(0), only_before.get(0, 1)),
            (3, 3, None)
        );
    }

    #[test]
    fn a_span_of_billions_of_columns_is_one_run() {
        let max = GRID_VALUE_MAX.to_string();
        let specs = [row(&max, &[(&max, VMerge::None), ("1", VMerge::None)])];
        let places = GridPlaces::of(&Grid::build(&specs, 2));
        assert_eq!(places.runs(0).len(), 2);
        assert_eq!(places.get(0, 2_147_483_644), None);
        assert_eq!(places.get(0, 4_000_000_000), Some((0, 2_147_483_645)));
        assert_eq!(places.get(0, 4_294_967_290), Some((0, 4_294_967_290)));
        assert_eq!(places.width(), 4_294_967_291);
    }
}
