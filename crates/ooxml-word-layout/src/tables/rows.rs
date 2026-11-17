//! Owns: a table's rows settled once their cells are measured (docx2svg's
//! `table.flow_table`, its rows' part): each row's flow from its cells, the edges between
//! rows, every row's height, and merges down made tall enough for their cells.
//!
//! Must not: measure a cell (`build` does) or place rows on pages (`place` does).

use super::build::flag;
use super::cells::{row_height, CellFlow, Merge, RowFlow, RowHeight};
use super::place::MergeDown;
use crate::px::Px;
use ooxml_word_model::resolve::ResolvedRow;
use ooxml_word_model::value::Value;

/// A row's flow from its measured cells (its edges and height are settled after).
pub(super) fn row_flow(row: &ResolvedRow, cells: Vec<CellFlow>) -> RowFlow {
    let stated = match row.properties.get("trHeight") {
        Some(Value::Height(value, rule)) => RowHeight {
            value: *value,
            exact: &**rule == "exact",
        },
        _ => RowHeight { value: 0, exact: false },
    };
    RowFlow {
        cells,
        top_border: Px::ZERO,
        height: Px::ZERO,
        stated,
        cant_split: flag(&row.properties, "cantSplit"),
        header: flag(&row.properties, "tblHeader"),
    }
}

/// The border on the edge between two rows, px: the widest stated there by the cells on
/// either side (`_edge_width`).
pub(super) fn edge_width(above: &[(i64, i64)], below: &[(i64, i64)]) -> Px {
    above
        .iter()
        .map(|(_, bottom)| *bottom)
        .chain(below.iter().map(|(top, _)| *top))
        .map(Px::twips)
        .max()
        .unwrap_or(Px::ZERO)
}

/// Each row's top border and height, and each merge down's rows made tall enough for its
/// cell (`flow_table`'s last part); the merges down over more than one row.
pub(super) fn settle_rows(rows: &mut [RowFlow]) -> Vec<MergeDown> {
    for r in 0..rows.len() {
        let above = r
            .checked_sub(1)
            .and_then(|k| rows.get(k))
            .map(RowFlow::cells_borders)
            .unwrap_or_default();
        let Some(row) = rows.get_mut(r) else { continue };
        let below: Vec<(i64, i64)> = row
            .cells
            .iter()
            .zip(row.cells_borders())
            .filter(|(c, _)| !c.merged())
            .map(|(_, b)| b)
            .collect();
        row.top_border = edge_width(&above, &below);
        row.height = row_height(row);
    }
    let mut merges = Vec::new();
    for r in 0..rows.len() {
        let count = rows.get(r).map_or(0, |row| row.cells.len());
        for c in 0..count {
            let Some(cell) = rows.get(r).and_then(|row| row.cells.get(c)) else {
                continue;
            };
            if cell.merge != Merge::Restart {
                continue;
            }
            let column = cell.column;
            let mut last = r;
            while rows
                .get(last + 1)
                .is_some_and(|next| next.cells.iter().any(|o| o.merged() && o.column == column))
            {
                last += 1;
            }
            let needed = cell.content + cell.margins();
            let heights: Px = rows.get(r..=last).unwrap_or_default().iter().map(|x| x.height).sum();
            let borders: Px = rows
                .get(r + 1..=last)
                .unwrap_or_default()
                .iter()
                .map(|x| x.top_border)
                .sum();
            let mut available = heights + borders;
            if needed > available {
                if let Some(bottom) = rows.get_mut(last) {
                    bottom.height += needed - available;
                }
                available = needed;
            }
            if let Some(cell) = rows.get_mut(r).and_then(|row| row.cells.get_mut(c)) {
                cell.rows = last - r + 1;
                cell.span_height = Some(available);
            }
            if last > r {
                merges.push(MergeDown {
                    row: r,
                    cell: c,
                    last,
                    column,
                });
            }
        }
    }
    merges
}
