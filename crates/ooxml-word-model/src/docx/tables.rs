//! Owns: a table's grid as docx-agent reads it (`edit/document.py`, `Table.grid`): each
//! row's cells (inside row-level content controls and custom XML too) placed by
//! `w:gridBefore` and `w:gridSpan`, a vertically merged cell answering at each position it
//! covers with its origin; the grid's column count (`w:tblGrid`); the rows accepting the
//! tracked changes keeps, and the columns (`accepted_columns`, Word's Accept All: a
//! deleted cell whose content is kept widens the cell before it, a grid column only removed
//! cells cover goes); a cell's text; a cell found by labels (`labels`).
//!
//! Must not: change a table or resolve its formatting.

use crate::docx::text::{paragraph_text, View};
use crate::docx::{at_path, find_path, StoryPart};
use crate::ids::{TableId, W};
use crate::xmlread::py_int;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

mod accepted;
mod labels;
mod merge_heads;
pub use accepted::{Accepted, Axis, Misplaced};
pub use labels::{collapse_space, no_cell_text};
pub use merge_heads::{cell_merge, merge_heads, VMerge};

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

    /// How many grid columns accepting the tracked changes keeps (docx-agent's
    /// `accepted_columns`).
    #[must_use]
    pub fn accepted_columns(&self) -> usize {
        self.kept_columns().len()
    }

    /// The grid columns accepting the tracked changes keeps, in order (docx-agent's
    /// `accepted_columns`' first value).
    #[must_use]
    pub fn kept_columns(&self) -> Vec<usize> {
        self.accepted_layout().kept
    }

    /// How many rows accepting the tracked changes keeps: every row not deleted as one.
    #[must_use]
    pub fn accepted_rows(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| find_path(r, &["trPr", "del"]).is_none())
            .count()
    }

    /// The number of rows.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.grid.len()
    }

    /// The paragraphs (indexes among the part's) of the cell over grid position `(row,
    /// column)`, those of tables nested in it included, in document order; none where no
    /// cell is there (docx-agent's `Table.cell(row, column).paragraphs`).
    #[must_use]
    pub fn cell_paragraphs(&self, row: usize, column: usize) -> Vec<usize> {
        let Some(origin) = self.grid.get(row).and_then(|r| r.get(column)).copied().flatten() else {
            return Vec::new();
        };
        let Some((_, path)) = self.cells.get(&origin) else {
            return Vec::new();
        };
        self.part.ids.paragraphs_within(path).iter().map(|p| p.index).collect()
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
            .paragraphs_within(path)
            .iter()
            .map(|p| paragraph_text(&p.element, View::Current))
            .collect();
        Some(paragraphs.join("\n"))
    }
}

/// A row's cells (row-level wrappers seen through) with the grid column each starts at and
/// its span.
fn placed_cells(row: &Element) -> Vec<(Arc<Element>, usize, usize)> {
    cells_at(row, &[])
        .into_iter()
        .map(|cell| (cell.element, cell.column, cell.span))
        .collect()
}

/// A cell of a row, placed.
#[derive(Debug, Clone)]
pub struct PlacedCell {
    /// The `w:tc`.
    pub element: Arc<Element>,
    /// Its place in its part.
    pub path: Box<[u32]>,
    /// The grid column it starts at (`w:gridBefore` and the spans before it counted).
    pub column: usize,
    /// Its span (`w:gridSpan`, at least 1).
    pub span: usize,
}

/// The cells of the row at `path` (row-level wrappers seen through), each with its place,
/// the grid column it starts at and its span.
#[must_use]
pub fn cells_at(row: &Element, path: &[u32]) -> Vec<PlacedCell> {
    let mut column = usize::try_from(int_val(find_path(row, &["trPr", "gridBefore"]), 0)).unwrap_or(0);
    let mut out = Vec::new();
    for (element, path) in cells_of(row, path) {
        let span = usize::try_from(int_val(find_path(&element, &["tcPr", "gridSpan"]), 1).max(1)).unwrap_or(1);
        out.push(PlacedCell {
            element,
            path,
            column,
            span,
        });
        column += span;
    }
    out
}

/// A table's own rows (inside row-level wrappers too, not its nested tables'), each with its
/// place below the table (docx-agent's `own_table_rows`).
#[must_use]
pub fn own_rows(table: &Element) -> Vec<(Arc<Element>, Vec<u32>)> {
    fn walk(element: &Element, at: &[u32], out: &mut Vec<(Arc<Element>, Vec<u32>)>) {
        for (k, node) in element.children().iter().enumerate() {
            let Some(child) = node.as_element() else { continue };
            let place = [at, &[u32::try_from(k).unwrap_or(u32::MAX)]].concat();
            if child.name().is(Some(W), "tr") {
                out.push((Arc::clone(child), place));
            } else if !child.name().is(Some(W), "tbl") {
                walk(child, &place, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(table, &[], &mut out);
    out
}

/// The cells of the table whose own rows are at `rows` (in `part`) Word's Accept All removes
/// with their text, as
/// `(row, cell)` (docx-agent's `cells_lost_on_accept`): in each row not itself deleted, a
/// cell with no next cell kept in its row whose last paragraph mark is a pending deletion,
/// not taken into a cell before it.
#[must_use]
pub fn cells_lost_on_accept(part: &StoryPart, rows: &[crate::ids::Path]) -> Vec<(usize, PlacedCell)> {
    let mut out = Vec::new();
    for (r, path) in rows.iter().enumerate() {
        let Some(row) = at_path(part.root(), path) else {
            continue;
        };
        if find_path(row, &["trPr", "del"]).is_some() {
            continue;
        }
        let shown: Vec<PlacedCell> = cells_at(row, path)
            .into_iter()
            .filter(|cell| !cell_deleted(&cell.element))
            .collect();
        let cells: Vec<&Element> = shown.iter().map(|cell| &*cell.element).collect();
        let (groups, gone) = crate::read::cell_ends(&cells);
        let taken: std::collections::HashSet<usize> = groups.iter().flat_map(|g| g.iter().skip(1).copied()).collect();
        for (n, cell) in shown.into_iter().enumerate() {
            if gone.contains(&n) && !taken.contains(&n) {
                out.push((r, cell));
            }
        }
    }
    out
}

fn cell_deleted(cell: &Element) -> bool {
    find_path(cell, &["tcPr", "cellDel"]).is_some()
}

/// Run content a reader sees (docx-agent's `review._SHOWN`).
const SHOWN: [&str; 10] = [
    "t",
    "tab",
    "br",
    "cr",
    "drawing",
    "pict",
    "object",
    "sym",
    "noBreakHyphen",
    "ptab",
];

/// Whether a deleted cell keeps anything (docx-agent's `_content_kept`): content outside
/// deletions and moves away, or a paragraph mark of its own not deleted.
pub(crate) fn content_kept(cell: &Element) -> bool {
    fn shown(node: &Element, deleted: bool) -> bool {
        node.child_elements().any(|e| {
            let gone = deleted || e.name().is(Some(W), "del") || e.name().is(Some(W), "moveFrom");
            let visible = e.name().namespace() == Some(W)
                && SHOWN.contains(&e.name().local())
                && !(e.name().local() == "t" && crate::docx::text::leading_text(e).is_empty());
            (visible && !gone) || shown(e, gone)
        })
    }
    fn marks(node: &Element, own: bool) -> bool {
        node.child_elements().any(|e| {
            if e.name().is(Some(W), "tc") {
                return false;
            }
            if own && e.name().is(Some(W), "p") {
                let mark = find_path(e, &["pPr", "rPr"]);
                let record = mark.and_then(|m| {
                    m.child_elements().find(|c| {
                        ["ins", "del", "moveFrom", "moveTo"]
                            .iter()
                            .any(|t| c.name().is(Some(W), t))
                    })
                });
                let deleted = mark.is_some_and(|m| {
                    m.child_elements()
                        .any(|c| c.name().is(Some(W), "del") || c.name().is(Some(W), "moveFrom"))
                });
                if record.is_none() || !deleted {
                    return true;
                }
            }
            marks(e, own)
        })
    }
    shown(cell, false) || marks(cell, true)
}
