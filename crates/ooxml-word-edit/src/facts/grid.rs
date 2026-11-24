//! Owns: `check`'s `table_grid_mismatch` (docx-agent's `layout_facts.table_grid_mismatch`,
//! round 53 R107, R124, R125, R128): the main story's tables (nested ones too) with a row
//! whose cells -- `w:gridBefore`, their spans, `w:gridAfter` -- do not reach exactly across
//! the table's `w:tblGrid` (Word draws it widened to the row, or short), naming the tracked
//! changes that put it there and the review that settles only them; and the tables ragged
//! as Accept All leaves them (rows ending on other grid columns than the widest).
//!
//! Must not: change the document (a review is tried on a copy, [`super::scratch`]); name
//! the session's call that made a table so (`made_by` is the session's record).

use super::scratch::settled;
use super::shapes::{Kept, KEPT};
use crate::names::W;
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::docx::block_facts::BlockFact;
use ooxml_word_model::docx::formatting::Formatting;
use ooxml_word_model::docx::revisions::records;
use ooxml_word_model::docx::tables::{cells_at, own_rows};
use ooxml_word_model::docx::{at_path, find_path};
use ooxml_word_model::ids::PlacedTable;
use ooxml_word_model::xmlread::py_int;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// Tracked cell and grid records: a table holding one may be off its grid by Word's design
/// until they are accepted.
const TRACKED_CELLS: [&str; 6] = [
    "cellIns",
    "cellDel",
    "cellMerge",
    "tcPrChange",
    "tblGridChange",
    "trPrChange",
];

/// Records whose acceptance changes which cells a table's rows keep.
const ACCEPT_RESHAPES: [&str; 3] = ["cellDel", "cellIns", "cellMerge"];

/// A row whose cells do not reach exactly across the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffGridRow {
    /// The row, from 0 among the table's own rows.
    pub row: usize,
    /// The grid columns its cells reach.
    pub reaches: i64,
}

/// A row of a ragged table that covers other grid columns than the widest rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaggedRow {
    /// The row, from 0.
    pub row: usize,
    /// The first and last grid column its cells cover.
    pub covers: (usize, usize),
}

/// A ragged table's shape: the grid columns its widest rows cover, and its other rows.
type Shape = (usize, Vec<RaggedRow>);

/// The review that settles only the changes that put a table off its grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridFix {
    /// Accept them (else reject: tried first).
    pub accept: bool,
    /// The changes.
    pub ids: Vec<String>,
}

/// One entry of `table_grid_mismatch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GridMismatch {
    /// A table with rows off its grid.
    OffGrid {
        /// The table.
        table: String,
        /// Its grid columns.
        grid: i64,
        /// Its rows off the grid (all of them).
        rows: Vec<OffGridRow>,
        /// Whether a row reaches past the grid (Word draws the table widened), else short.
        widened: bool,
        /// Whether the table holds tracked cell or grid records.
        tracked: bool,
        /// The tracked changes in its rows off the grid and its grid's own record, as
        /// `word_changes` names them (none where none can be named).
        changes: Vec<String>,
        /// The review that settles only them, where one does.
        fix: Option<GridFix>,
    },
    /// A table whose rows do not line up (as Accept All leaves it, where pending changes
    /// reshape it).
    Ragged {
        /// The table.
        table: String,
        /// The grid columns its widest rows cover.
        grid: usize,
        /// The other rows.
        rows: Vec<RaggedRow>,
        /// Whether pending tracked changes make the shape.
        after_accept: bool,
    },
}

/// `check`'s `table_grid_mismatch` of `doc`: the tables off their grid in document order,
/// then the ragged ones. `keep`: only the tables whose id it keeps are read.
///
/// # Errors
/// When the version cannot be read.
pub fn table_grid_mismatch(doc: &WordDoc, keep: Option<&dyn Fn(&str) -> bool>) -> Result<Vec<GridMismatch>> {
    let docx = doc.docx()?;
    let body = docx.body().ok_or_else(|| EditError::NoPart(docx.main.clone()))?;
    // Each table found by its element and place; its id named only for an entry (or `keep`).
    let named = |table: &PlacedTable<'_>| body.ids.tables().head(table.ordinal).map(|t| t.id).unwrap_or_default();
    let tables: Vec<PlacedTable<'_>> = body
        .ids
        .tables()
        .placed()
        .filter(|t| keep.is_none_or(|k| k(&named(t))))
        .collect();
    let formatting = Formatting::read(&docx);
    let shapes: Vec<Arc<TableShape>> = tables
        .iter()
        .map(|table| {
            body.block_fact::<Shapes>(&formatting, table.element, &table.path, || {
                Arc::new(TableShape::of(doc, table))
            })
        })
        .collect();
    let mut by_place: Option<HashMap<Box<[u32]>, String>> = None;
    let mut found = Vec::new();
    for (table, shape) in tables.iter().zip(&shapes) {
        let (grid, off, tracked) = (shape.grid, shape.off.clone(), shape.tracked);
        if off.is_empty() {
            continue;
        }
        let (changes, fix) = if tracked {
            let named = by_place.get_or_insert_with(|| {
                records(&docx)
                    .into_iter()
                    .filter(|r| r.part == body.part)
                    .map(|r| (r.path, r.id))
                    .collect()
            });
            settled_by(doc, table, &off, named)
        } else {
            (Vec::new(), None)
        };
        found.push(GridMismatch::OffGrid {
            table: named(table),
            grid,
            widened: off.iter().any(|r| r.reaches > grid),
            rows: off,
            tracked,
            changes,
            fix,
        });
    }
    for (table, shape) in tables.iter().zip(&shapes) {
        let Some(ragged) = &shape.ragged else {
            continue;
        };
        // A row off the grid is said above; as it is now, that is the ragged shape too.
        if ragged.after_accept || shape.off.is_empty() {
            found.push(GridMismatch::Ragged {
                table: named(table),
                grid: ragged.grid,
                rows: ragged.rows.clone(),
                after_accept: ragged.after_accept,
            });
        }
    }
    Ok(found)
}

/// A table's shape as `table_grid_mismatch` reads it: its grid columns, its rows off the
/// grid, whether it holds tracked cell or grid records, and its ragged shape
/// ([`ragged_rows`]). Read from the table alone; the block tier keeps it across versions
/// ([`Shapes`]).
struct TableShape {
    grid: i64,
    off: Vec<OffGridRow>,
    tracked: bool,
    ragged: Option<Ragged>,
}

impl TableShape {
    /// `table`'s shape.
    fn of(doc: &WordDoc, table: &PlacedTable<'_>) -> TableShape {
        let rows: Vec<Arc<Element>> = table
            .rows
            .iter()
            .filter_map(|path| at_path(table.element, path.get(table.path.len()..)?).cloned())
            .collect();
        let (grid, off) = off_grid_rows(table.element, &rows);
        TableShape {
            grid,
            tracked: !off.is_empty() && has_any(table.element, &TRACKED_CELLS),
            off,
            ragged: ragged_rows(doc, table.element),
        }
    }
}

/// A table's [`TableShape`] as the block tier keeps it ([`BlockFact`]): by the table, the
/// outermost table holding it and the formatting parts.
struct Shapes;

impl BlockFact for Shapes {
    type Value = Arc<TableShape>;

    fn weight(value: &Self::Value) -> usize {
        std::mem::size_of::<TableShape>()
            + value.off.len() * std::mem::size_of::<OffGridRow>()
            + value
                .ragged
                .as_ref()
                .map_or(0, |r| r.rows.len() * std::mem::size_of::<RaggedRow>())
    }
}

/// A ragged table's shape as Accept All leaves it ([`ragged_rows`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ragged {
    /// The grid columns its widest rows cover.
    pub grid: usize,
    /// Its other rows.
    pub rows: Vec<RaggedRow>,
    /// Whether pending tracked changes make the shape.
    pub after_accept: bool,
}

/// `table` as Accept All leaves it when its rows' cells do not all start and end on the
/// same grid columns, off its grid or not (docx-agent's `ragged_rows`; check's
/// `table_grid_mismatch` says an off-grid table's present shape as off the grid instead);
/// `None` for a table whose rows line up.
#[must_use]
pub fn ragged_rows(doc: &WordDoc, table: &Arc<Element>) -> Option<Ragged> {
    let after_accept = reshaped_on_accept(table);
    let (grid, rows) = if after_accept {
        accepted_shape(doc, table)
    } else {
        shape(table)
    }?;
    Some(Ragged {
        grid,
        rows,
        after_accept,
    })
}

fn int_val(node: Option<&Element>, default: i64) -> i64 {
    node.and_then(|n| n.attribute(Some(W), "val"))
        .and_then(py_int)
        .unwrap_or(default)
}

/// A table's grid columns, and its rows (`rows`, its own, in order) whose cells -- with
/// `w:gridBefore` and `w:gridAfter` -- do not reach exactly across them.
fn off_grid_rows(table: &Element, rows: &[Arc<Element>]) -> (i64, Vec<OffGridRow>) {
    let count = table.find(Some(W), "tblGrid").map_or(0, |g| {
        g.child_elements().filter(|c| c.name().is(Some(W), "gridCol")).count()
    });
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    let mut out = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let mut reach =
            int_val(find_path(row, &["trPr", "gridBefore"]), 0) + int_val(find_path(row, &["trPr", "gridAfter"]), 0);
        for cell in cells_at(row, &[]) {
            reach += int_val(find_path(&cell.element, &["tcPr", "gridSpan"]), 1).max(1);
        }
        if reach != count {
            out.push(OffGridRow { row: r, reaches: reach });
        }
    }
    (count, out)
}

/// Whether any element below `element` is a `w:` element named in `locals`.
fn has_any(element: &Element, locals: &[&str]) -> bool {
    element
        .child_elements()
        .any(|c| (c.name().namespace() == Some(W) && locals.contains(&c.name().local())) || has_any(c, locals))
}

/// Every `w:` element named in `locals` below `element` (at `at`), with its place, in
/// document order.
fn below(element: &Element, at: &[u32], locals: &[&str], out: &mut Vec<Vec<u32>>) {
    for (k, node) in element.children().iter().enumerate() {
        let Some(child) = node.as_element() else { continue };
        let place = [at, &[u32::try_from(k).unwrap_or(u32::MAX)]].concat();
        if child.name().namespace() == Some(W) && locals.contains(&child.name().local()) {
            out.push(place.clone());
        }
        below(child, &place, locals, out);
    }
}

/// The tracked changes that put a table off its grid -- the cell and row records in its rows
/// off the grid and its grid's own record -- and the review that settles only them:
/// rejecting them where that leaves every row on the grid and lined up (tried on a copy),
/// else accepting them (docx-agent's `_grid_settled_by`).
fn settled_by(
    doc: &WordDoc,
    table: &PlacedTable<'_>,
    off: &[OffGridRow],
    named: &HashMap<Box<[u32]>, String>,
) -> (Vec<String>, Option<GridFix>) {
    let mut places: Vec<Vec<u32>> = Vec::new();
    for entry in off {
        if let Some(path) = table.rows.get(entry.row) {
            let Some(row) =
                ooxml_word_model::docx::at_path(table.element, path.get(table.path.len()..).unwrap_or_default())
            else {
                continue;
            };
            below(row, path, &TRACKED_CELLS, &mut places);
        }
    }
    let grid_at = table
        .element
        .children()
        .iter()
        .position(|n| n.as_element().is_some_and(|e| e.name().is(Some(W), "tblGrid")));
    if let Some(k) = grid_at.and_then(|k| u32::try_from(k).ok()) {
        let at = [&table.path[..], &[k]].concat();
        if let Some(grid) = table.element.find(Some(W), "tblGrid") {
            below(grid, &at, &["tblGridChange"], &mut places);
        }
    }
    let mut changes: Vec<String> = Vec::new();
    for place in &places {
        if let Some(id) = named.get(place.as_slice()) {
            if !changes.contains(id) {
                changes.push(id.clone());
            }
        }
    }
    if changes.is_empty() {
        return (changes, None);
    }
    let relative: Vec<Vec<u32>> = places
        .iter()
        .filter_map(|p| p.get(table.path.len()..).map(<[u32]>::to_vec))
        .collect();
    let fix = [false, true].into_iter().find_map(|accept| {
        let copy = settled(doc, table.element, accept, Some(&relative))?;
        let rows: Vec<Arc<Element>> = own_rows(&copy).into_iter().map(|(row, _)| row).collect();
        (off_grid_rows(&copy, &rows).1.is_empty() && shape(&copy).is_none()).then(|| GridFix {
            accept,
            ids: changes.clone(),
        })
    });
    (changes, fix)
}

/// Whether accepting `table`'s own tracked changes changes which cells its rows keep: a
/// cell deleted, inserted or merged down, a row deleted, or a cell's last paragraph mark
/// deleted (docx-agent's `_reshaped_on_accept`).
fn reshaped_on_accept(table: &Element) -> bool {
    fn own(element: &Element) -> bool {
        element.child_elements().any(|c| {
            if c.name().is(Some(W), "tbl") {
                return false;
            }
            let reshapes = c.name().namespace() == Some(W) && ACCEPT_RESHAPES.contains(&c.name().local());
            let row_deleted = c.name().is(Some(W), "trPr") && c.find(Some(W), "del").is_some();
            let end_deleted = c.name().is(Some(W), "tc")
                && c.child_elements()
                    .filter(|p| p.name().is(Some(W), "p"))
                    .last()
                    .is_some_and(|p| find_path(p, &["pPr", "rPr", "del"]).is_some());
            reshapes || row_deleted || end_deleted || own(c)
        })
    }
    own(table)
}

/// A table's shape ([`shape`]) as Accept All leaves it: read from a copy with its tracked
/// changes accepted, kept by the table's content ([`Kept`]) -- a table no edit touched is
/// not settled again by every check (docx-agent's `ragged_rows`, round 57 R124).
fn accepted_shape(doc: &WordDoc, table: &Arc<Element>) -> Option<Shape> {
    static ACCEPTED: OnceLock<Kept<Option<Shape>>> = OnceLock::new();
    ACCEPTED
        .get_or_init(|| Kept::new(KEPT))
        .get_or(table, || settled(doc, table, true, None).and_then(|t| shape(&t)))
}

/// A table's columns covered by its widest rows and the rows that cover others, or `None`
/// where its rows line up (docx-agent's `_shape`).
fn shape(table: &Element) -> Option<Shape> {
    let mut extents: Vec<(usize, usize, usize)> = Vec::new();
    for (r, (row, _)) in own_rows(table).into_iter().enumerate() {
        let cells = cells_at(&row, &[]);
        if let (Some(first), Some(last)) = (cells.first(), cells.last()) {
            extents.push((r, first.column, (last.column + last.span).saturating_sub(1)));
        }
    }
    if extents.len() < 2 {
        return None;
    }
    let low = extents.iter().map(|e| e.1).min()?;
    let high = extents.iter().map(|e| e.2).max()?;
    let off: Vec<RaggedRow> = extents
        .into_iter()
        .filter(|(_, a, b)| (*a, *b) != (low, high))
        .map(|(row, a, b)| RaggedRow { row, covers: (a, b) })
        .collect();
    (!off.is_empty()).then(|| (high - low + 1, off))
}
