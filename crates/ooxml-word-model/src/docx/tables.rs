//! Owns: a table's grid as docx-agent reads it (`edit/document.py`, `Table.grid`): each
//! row's cells (inside row-level content controls and custom XML too) placed by
//! `w:gridBefore` and `w:gridSpan`, a vertically merged cell answering at each position it
//! covers with its origin; the grid's column count (`w:tblGrid`); the rows accepting the
//! tracked changes keeps; and a cell's text.
//!
//! Must not: change a table or resolve its formatting.

use crate::docx::text::{paragraph_text, View};
use crate::docx::{at_path, find_path, StoryPart};
use crate::ids::{TableId, W};
use crate::xmlread::py_int;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// A cell element with its place in its part.
type Placed = (Arc<Element>, Box<[u32]>);

/// A table's grid.
pub struct TableGrid<'a> {
    part: &'a StoryPart,
    table: Arc<Element>,
    rows: Vec<Arc<Element>>,
    /// Per row, per grid position: the origin `(row, column)` of the cell there.
    grid: Vec<Vec<Option<(usize, usize)>>>,
    /// The cell element at each origin, with its place.
    cells: HashMap<(usize, usize), Placed>,
}

fn int_val(node: Option<&Element>, default: i64) -> i64 {
    node.and_then(|n| n.attribute(Some(W), "val"))
        .and_then(py_int)
        .unwrap_or(default)
}

/// A row's `w:tc`, with their places, also inside row-level `w:sdt` and `w:customXml`.
fn cells_of(row: &Element, base: &[u32]) -> Vec<(Arc<Element>, Box<[u32]>)> {
    let mut out = Vec::new();
    for (k, node) in row.children().iter().enumerate() {
        let Some(child) = node.as_element() else { continue };
        let mut path = base.to_vec();
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        if child.name().is(Some(W), "tc") {
            out.push((Arc::clone(child), path.into_boxed_slice()));
        } else if child.name().is(Some(W), "sdt") {
            if let Some((j, content)) = child.children().iter().enumerate().find_map(|(j, n)| {
                n.as_element()
                    .filter(|e| e.name().is(Some(W), "sdtContent"))
                    .map(|e| (j, e))
            }) {
                path.push(u32::try_from(j).unwrap_or(u32::MAX));
                out.extend(cells_of(content, &path));
            }
        } else if child.name().is(Some(W), "customXml") {
            out.extend(cells_of(child, &path));
        }
    }
    out
}

impl<'a> TableGrid<'a> {
    /// The grid of `table`, a table of `part`.
    #[must_use]
    pub fn new(part: &'a StoryPart, table: &TableId) -> TableGrid<'a> {
        let mut out = TableGrid {
            part,
            table: Arc::clone(&table.element),
            rows: Vec::new(),
            grid: Vec::new(),
            cells: HashMap::new(),
        };
        let mut origins: HashMap<usize, (usize, usize)> = HashMap::new();
        for (r, (path, _)) in table.rows.iter().enumerate() {
            let Some(row) = at_path(part.root(), path) else {
                continue;
            };
            out.rows.push(Arc::clone(row));
            let before = usize::try_from(int_val(find_path(row, &["trPr", "gridBefore"]), 0)).unwrap_or(0);
            let mut line = vec![None; before];
            let mut column = before;
            for (tc, cell_path) in cells_of(row, path) {
                let span = usize::try_from(int_val(find_path(&tc, &["tcPr", "gridSpan"]), 1).max(1)).unwrap_or(1);
                let merge = find_path(&tc, &["tcPr", "vMerge"]);
                let continues = merge.is_some_and(|m| matches!(m.attribute(Some(W), "val"), None | Some("continue")));
                let origin = match (continues, origins.get(&column)) {
                    (true, Some(origin)) => *origin,
                    _ => {
                        for c in column..column + span {
                            origins.insert(c, (r, column));
                        }
                        out.cells.insert((r, column), (Arc::clone(&tc), cell_path));
                        (r, column)
                    }
                };
                line.extend(std::iter::repeat_n(Some(origin), span));
                column += span;
            }
            out.grid.push(line);
        }
        out
    }

    /// The number of grid columns (`w:gridCol` in `w:tblGrid`).
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.table.find(Some(W), "tblGrid").map_or(0, |g| {
            g.child_elements().filter(|c| c.name().is(Some(W), "gridCol")).count()
        })
    }

    /// How many rows accepting the tracked changes keeps: every row not deleted as one.
    #[must_use]
    pub fn accepted_rows(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| find_path(r, &["trPr", "del"]).is_none())
            .count()
    }

    /// The text of the cell over grid position `(row, column)`: its paragraphs' text in the
    /// current view, one line each; `None` where no cell is there.
    #[must_use]
    pub fn cell_text(&self, row: usize, column: usize) -> Option<String> {
        let origin = (*self.grid.get(row)?.get(column)?)?;
        let (_, path) = self.cells.get(&origin)?;
        let paragraphs: Vec<String> = self
            .part
            .ids
            .paragraphs
            .iter()
            .filter(|p| p.path.starts_with(path))
            .map(|p| paragraph_text(&p.element, View::Current))
            .collect();
        Some(paragraphs.join("\n"))
    }
}
