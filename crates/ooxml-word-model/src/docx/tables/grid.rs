//! Owns: [`Grid`], one table's grid as Word places it, built from plain values
//! ([`RowSpec`]): each cell's first grid column and span, the vertical merges' origins, the
//! grid's width and each row's coverage of `w:tblGrid`; the value rule ([`grid_value`]) and
//! the cell walk ([`row_cells`]) every reader of a table's grid shares, and the adapter from
//! a row element ([`RowSpec::of`]).
//!
//! The rules (Word 16.106, `observations/grid-values-word.json`):
//! - a `w:gridSpan`, `w:gridBefore` or `w:gridAfter` value is an `ST_DecimalNumber`: an
//!   optional sign and decimal digits, nothing else (Word does not open a document with a
//!   span of ` 2`, `1_0` or `-1`, or a `w:gridBefore` of `-1`); anything else is absent. A span
//!   below 1 is 1 (Word places a span of `0` as one column); a count below 0 is 0 (a
//!   `w:gridAfter` of `-1`). The raw span is kept ([`CellSpec::span_raw`]), so validation still
//!   reports one that is not valid;
//! - a row's cells are its `w:tc`, also inside `w:sdt`/`w:sdtContent` and `w:customXml`, at
//!   any depth, never inside a cell or a nested table;
//! - a cell continuing a vertical merge (`w:vMerge` without `restart`) belongs to the cell
//!   whose grid column it starts at in the rows above (its origin); with none there (the
//!   table's first row) it is an origin itself, drawn in its own place as Word draws it.
//!
//! Must not: read anything but the values it is given (the elements are read only by the
//! adapters), or decide what a view of the table keeps.

use super::merge_heads::{cell_merge, VMerge};
use crate::docx::find_path;
use crate::ids::W;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// A grid value (`w:gridSpan`, `w:gridBefore`, `w:gridAfter`) as Word reads it: an
/// `ST_DecimalNumber` -- an optional `+` or `-` and decimal digits, no white space -- else
/// `None`.
#[must_use]
pub fn grid_value(raw: &str) -> Option<i64> {
    let digits = raw.strip_prefix(['+', '-']).unwrap_or(raw);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse::<i64>().ok()
}

/// A count of grid columns (`w:gridBefore`, `w:gridAfter`): absent or not valid is 0, below
/// 0 is 0.
fn count(raw: Option<&str>) -> usize {
    raw.and_then(grid_value)
        .map_or(0, |v| usize::try_from(v.max(0)).unwrap_or(usize::MAX))
}

/// One cell's values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellSpec {
    /// `w:gridSpan`'s value as written, where there is one.
    pub span_raw: Option<Box<str>>,
    /// The span ([`grid_value`], at least 1).
    pub span: usize,
    /// Its part in a vertical merge.
    pub vmerge: VMerge,
}

impl CellSpec {
    /// A cell of `w:gridSpan` `span_raw` (as written) and vertical merge `vmerge`.
    #[must_use]
    pub fn new(span_raw: Option<&str>, vmerge: VMerge) -> CellSpec {
        let span = span_raw
            .and_then(grid_value)
            .map_or(1, |v| usize::try_from(v.max(1)).unwrap_or(usize::MAX));
        CellSpec {
            span_raw: span_raw.map(Into::into),
            span,
            vmerge,
        }
    }

    /// Whether its span is valid: absent, or an [`grid_value`] of at least 1.
    #[must_use]
    pub fn span_valid(&self) -> bool {
        self.span_raw
            .as_deref()
            .is_none_or(|raw| grid_value(raw).is_some_and(|v| v >= 1))
    }

    /// The values of the cell `tc`.
    #[must_use]
    pub fn of(tc: &Element) -> CellSpec {
        let raw = find_path(tc, &["tcPr", "gridSpan"]).and_then(|g| g.attribute(Some(W), "val"));
        CellSpec::new(raw, cell_merge(tc, false))
    }
}

/// One row's values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowSpec {
    /// Grid columns before its first cell (`w:gridBefore`).
    pub before: usize,
    /// Grid columns after its last cell (`w:gridAfter`).
    pub after: usize,
    /// Its cells, in order.
    pub cells: Vec<CellSpec>,
}

impl RowSpec {
    /// A row of `w:gridBefore` / `w:gridAfter` values as written (absent: `None`).
    #[must_use]
    pub fn new(before_raw: Option<&str>, after_raw: Option<&str>, cells: Vec<CellSpec>) -> RowSpec {
        RowSpec {
            before: count(before_raw),
            after: count(after_raw),
            cells,
        }
    }

    /// The values of the row `tr` (its cells by [`row_cells`]).
    #[must_use]
    pub fn of(tr: &Element) -> RowSpec {
        let value = |name: &str| find_path(tr, &["trPr", name]).and_then(|g| g.attribute(Some(W), "val"));
        let cells = row_cells(tr).iter().map(|(tc, _)| CellSpec::of(tc)).collect();
        RowSpec::new(value("gridBefore"), value("gridAfter"), cells)
    }
}

/// A row's cells: its `w:tc`, also inside `w:sdt`/`w:sdtContent` and `w:customXml` at any
/// depth (never inside a cell or a nested table), each with its place below the row (child
/// indexes).
#[must_use]
pub fn row_cells(row: &Element) -> Vec<(Arc<Element>, Box<[u32]>)> {
    fn walk(element: &Element, at: &mut Vec<u32>, out: &mut Vec<(Arc<Element>, Box<[u32]>)>) {
        for (k, node) in element.children().iter().enumerate() {
            let Some(child) = node.as_element() else { continue };
            at.push(u32::try_from(k).unwrap_or(u32::MAX));
            let name = child.name();
            if name.is(Some(W), "tc") {
                out.push((Arc::clone(child), at.clone().into_boxed_slice()));
            } else if ["sdt", "sdtContent", "customXml"].iter().any(|n| name.is(Some(W), n)) {
                walk(child, at, out);
            }
            at.pop();
        }
    }
    let mut out = Vec::new();
    walk(row, &mut Vec::new(), &mut out);
    out
}

/// A row's cells ([`row_cells`]) placed on the grid by the row alone (no merges): each with
/// its place below the row, first grid column and span.
#[must_use]
pub fn placed_row_cells(row: &Element) -> Vec<(Arc<Element>, Box<[u32]>, GridCell)> {
    let grid = Grid::build(std::slice::from_ref(&RowSpec::of(row)), 0);
    let placed = grid.rows.into_iter().next().map(|r| r.cells).unwrap_or_default();
    row_cells(row)
        .into_iter()
        .zip(placed)
        .map(|((tc, place), cell)| (tc, place, cell))
        .collect()
}

/// A cell placed on the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridCell {
    /// The grid column it starts at.
    pub first: usize,
    /// How many grid columns it covers.
    pub span: usize,
    /// The cell its vertical merge starts at, `(row, cell)`: itself where it starts one or
    /// is in none.
    pub origin: (usize, usize),
}

impl GridCell {
    /// The grid columns it covers.
    #[must_use]
    pub fn columns(&self) -> std::ops::Range<usize> {
        self.first..self.first + self.span
    }
}

/// How a row's grid columns (before, cells, after) meet `w:tblGrid`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// As many.
    Exact,
    /// This many fewer.
    Short(usize),
    /// This many more.
    Over(usize),
}

/// A row on the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridRow {
    /// Grid columns before its first cell.
    pub before: usize,
    /// Grid columns after its last cell.
    pub after: usize,
    /// Its cells, in order.
    pub cells: Vec<GridCell>,
    /// The grid column after its last cell.
    pub end: usize,
    /// Its columns (before, cells, after) against `w:tblGrid`'s.
    pub coverage: Coverage,
}

/// A table's grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    /// Its rows.
    pub rows: Vec<GridRow>,
    /// The widest row's grid columns, `w:gridAfter` counted.
    pub width: usize,
    /// `w:tblGrid`'s columns.
    pub columns: usize,
}

impl Grid {
    /// The grid of `rows` in a table whose `w:tblGrid` has `columns` columns.
    #[must_use]
    pub fn build(rows: &[RowSpec], columns: usize) -> Grid {
        let mut origins: HashMap<usize, (usize, usize)> = HashMap::new();
        let mut out = Vec::with_capacity(rows.len());
        for (r, row) in rows.iter().enumerate() {
            let mut column = row.before;
            let mut cells = Vec::with_capacity(row.cells.len());
            for (k, cell) in row.cells.iter().enumerate() {
                let continued = match cell.vmerge {
                    VMerge::Continue => origins.get(&column).copied(),
                    VMerge::None | VMerge::Restart => None,
                };
                let origin = continued.unwrap_or_else(|| {
                    for c in column..column.saturating_add(cell.span) {
                        origins.insert(c, (r, k));
                    }
                    (r, k)
                });
                cells.push(GridCell {
                    first: column,
                    span: cell.span,
                    origin,
                });
                column = column.saturating_add(cell.span);
            }
            let total = column.saturating_add(row.after);
            let coverage = match total.cmp(&columns) {
                std::cmp::Ordering::Equal => Coverage::Exact,
                std::cmp::Ordering::Less => Coverage::Short(columns - total),
                std::cmp::Ordering::Greater => Coverage::Over(total - columns),
            };
            out.push(GridRow {
                before: row.before,
                after: row.after,
                cells,
                end: column,
                coverage,
            });
        }
        let width = out.iter().map(|r| r.end.saturating_add(r.after)).max().unwrap_or(0);
        Grid {
            rows: out,
            width,
            columns,
        }
    }

    /// The grid of the rows `rows` of `table` (`w:tblGrid` read from `table`).
    #[must_use]
    pub fn of(table: &Element, rows: &[&Element]) -> Grid {
        let specs: Vec<RowSpec> = rows.iter().map(|r| RowSpec::of(r)).collect();
        Grid::build(&specs, grid_columns(table))
    }

    /// The cell `(row, cell)` covering grid position `(row, column)`, where there is one.
    #[must_use]
    pub fn cell_at(&self, row: usize, column: usize) -> Option<(usize, usize)> {
        let line = self.rows.get(row)?;
        let k = line.cells.iter().position(|c| c.columns().contains(&column))?;
        Some((row, k))
    }

    /// The origin `(row, cell)` of the cell covering grid position `(row, column)`.
    #[must_use]
    pub fn origin_at(&self, row: usize, column: usize) -> Option<(usize, usize)> {
        let (r, k) = self.cell_at(row, column)?;
        self.rows.get(r)?.cells.get(k).map(|c| c.origin)
    }
}

/// The number of `w:gridCol` in `table`'s `w:tblGrid`.
#[must_use]
pub fn grid_columns(table: &Element) -> usize {
    table.find(Some(W), "tblGrid").map_or(0, |g| {
        g.child_elements().filter(|c| c.name().is(Some(W), "gridCol")).count()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_values_are_decimal_numbers_as_word_opens_them() {
        assert_eq!(grid_value("2"), Some(2));
        assert_eq!(grid_value("+2"), Some(2));
        assert_eq!(grid_value("-1"), Some(-1));
        for refused in [" 2", "2 ", "1_0", "", "+", "x", "2.0"] {
            assert_eq!(grid_value(refused), None, "{refused:?}");
        }
        assert_eq!(CellSpec::new(Some("0"), VMerge::None).span, 1);
        assert!(!CellSpec::new(Some("0"), VMerge::None).span_valid());
        assert!(!CellSpec::new(Some(" 2"), VMerge::None).span_valid());
        assert!(CellSpec::new(None, VMerge::None).span_valid());
        assert_eq!(
            RowSpec::new(Some("-1"), Some("-1"), Vec::new()),
            RowSpec::new(None, None, Vec::new())
        );
    }

    #[test]
    fn a_grid_places_cells_follows_merges_and_measures_rows_against_the_grid() {
        let cell = |span: &str, vmerge| CellSpec::new(Some(span), vmerge);
        let rows = [
            RowSpec::new(Some("1"), None, vec![cell("2", VMerge::Restart)]),
            RowSpec::new(Some("1"), Some("1"), vec![cell("2", VMerge::Continue)]),
            RowSpec::new(None, None, vec![cell("1", VMerge::Continue), cell("3", VMerge::None)]),
        ];
        let grid = Grid::build(&rows, 3);
        assert_eq!(
            grid.rows[0].cells[0],
            GridCell {
                first: 1,
                span: 2,
                origin: (0, 0)
            }
        );
        assert_eq!(grid.rows[1].cells[0].origin, (0, 0));
        assert_eq!(
            grid.rows[2].cells[0].origin,
            (2, 0),
            "nothing merged starts at column 0"
        );
        assert_eq!(grid.rows[0].coverage, Coverage::Exact);
        assert_eq!(grid.rows[1].coverage, Coverage::Over(1));
        assert_eq!(grid.rows[2].coverage, Coverage::Over(1));
        assert_eq!(grid.width, 4);
        assert_eq!(grid.origin_at(1, 2), Some((0, 0)));
        assert_eq!(grid.cell_at(1, 0), None);
    }
}
