//! Owns: a table's shape on an editable copy, as docx-agent's `edit/table_format.py`
//! reads it: its own rows (not a nested table's, row-level content controls seen through),
//! each row's cells with their first grid column and span (`w:gridBefore`, `w:gridSpan`),
//! the cells sharing a cell's edges, the grid's widths, and a width shared in proportion.
//!
//! Must not: change anything but the grid it is asked to write.

use crate::names::W;
use crate::review::xml::{ancestor, find_path, insert_in_order, int, make};
use crate::validate::Orders;
use crate::work::{Id, Work};

/// A cell of a row: the cell, its first grid column and its span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placed {
    pub(crate) cell: Id,
    pub(crate) start: usize,
    pub(crate) span: usize,
}

/// `table`'s own rows, in order (docx-agent's `own_rows`).
pub(crate) fn own_rows(work: &Work, table: Id) -> Vec<Id> {
    work.iter(table, W, &["tr"])
        .into_iter()
        .filter(|row| ancestor(work, *row, "tbl") == Some(table))
        .collect()
}

/// A whole number `w:val` of the child at `path`, `default` where none or no number.
fn val(work: &Work, id: Id, path: &[&str], default: i64) -> i64 {
    int(work, find_path(work, id, path), "val", default)
}

/// `row`'s own cells, placed (docx-agent's `row_cells`).
pub(crate) fn row_cells(work: &Work, row: Id) -> Vec<Placed> {
    let mut column = usize::try_from(val(work, row, &["trPr", "gridBefore"], 0)).unwrap_or(0);
    let mut out = Vec::new();
    for cell in work.iter(row, W, &["tc"]) {
        if ancestor(work, cell, "tr") != Some(row) {
            continue;
        }
        let span = usize::try_from(val(work, cell, &["tcPr", "gridSpan"], 1).max(1)).unwrap_or(1);
        out.push(Placed {
            cell,
            start: column,
            span,
        });
        column += span;
    }
    out
}

/// The physical cell of row `row` over grid column `column` (docx-agent's `_cell_element`).
pub(crate) fn cell_over(work: &Work, table: Id, row: usize, column: usize) -> Option<Id> {
    let row = *own_rows(work, table).get(row)?;
    row_cells(work, row)
        .into_iter()
        .find(|p| p.start <= column && column < p.start + p.span)
        .map(|p| p.cell)
}

/// The cells sharing each edge of `cell` (where one cell does): top, bottom, left, right
/// (docx-agent's `_neighbours`).
pub(crate) fn neighbours(work: &Work, table: Id, cell: Id) -> [Option<Id>; 4] {
    let placed: Vec<(usize, Placed)> = own_rows(work, table)
        .into_iter()
        .enumerate()
        .flat_map(|(r, row)| row_cells(work, row).into_iter().map(move |p| (r, p)))
        .collect();
    let mut out = [None; 4];
    let Some(&(r, mine)) = placed.iter().find(|(_, p)| p.cell == cell) else {
        return out;
    };
    for &(rr, p) in &placed {
        let same = p.start == mine.start && p.span == mine.span;
        if rr == r && p.start + p.span == mine.start {
            out[2] = Some(p.cell);
        }
        if rr == r && p.start == mine.start + mine.span {
            out[3] = Some(p.cell);
        }
        if rr + 1 == r && same {
            out[0] = Some(p.cell);
        }
        if rr == r + 1 && same {
            out[1] = Some(p.cell);
        }
    }
    out
}

/// The grid's column widths, in twips (0 where a column states none).
pub(crate) fn grid_widths(work: &Work, table: Id) -> Vec<i64> {
    let Some(grid) = work.find(table, W, "tblGrid") else {
        return Vec::new();
    };
    work.children(grid)
        .iter()
        .filter(|c| work.is(**c, W, "gridCol"))
        .map(|c| int(work, Some(*c), "w", 0))
        .collect()
}

/// Write the grid's widths (docx-agent's `set_grid`).
pub(crate) fn set_grid(work: &mut Work, table: Id, widths: &[i64], orders: &Orders) {
    let grid = match work.find(table, W, "tblGrid") {
        Some(grid) => grid,
        None => {
            let grid = make(work, "tblGrid", &[]);
            insert_in_order(work, table, grid, orders);
            grid
        }
    };
    for column in work.children(grid).to_vec() {
        if work.is(column, W, "gridCol") {
            work.remove(column);
        }
    }
    for width in widths {
        let column = make(work, "gridCol", &[("w", &width.to_string())]);
        insert_in_order(work, grid, column, orders);
    }
}

/// `total` shared in proportion to `weights`, each grid line at the ceiling of its
/// cumulative share (docx-agent's `share`).
pub(crate) fn share(total: i64, weights: &[i64]) -> Vec<i64> {
    let sum: i64 = weights.iter().sum();
    let ones = vec![1; weights.len()];
    let (weights, whole) = if sum == 0 {
        (ones.as_slice(), i64::try_from(weights.len()).unwrap_or(1))
    } else {
        (weights, sum)
    };
    let mut out = Vec::with_capacity(weights.len());
    let (mut done, mut running) = (0i64, 0i64);
    for weight in weights {
        running += weight;
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let line = (total as f64 * running as f64 / whole as f64 - 1e-9).ceil() as i64;
        out.push(line - done);
        done = line;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::share;

    #[test]
    fn shares_as_word_does() {
        // Word's 80% of 9016 over three equal columns
        assert_eq!(share(7213, &[1, 1, 1]), [2405, 2404, 2404]);
        assert_eq!(share(100, &[0, 0]), [50, 50]);
    }
}
