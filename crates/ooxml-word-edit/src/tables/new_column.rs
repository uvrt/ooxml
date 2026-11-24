//! Owns: a grid column inserted as docx-agent's `edit/tables.py` `insert_column` inserts it
//! (Word's Insert Columns, measured), untracked and tracked: a grid column as wide as the
//! one beside it, a fixed table width grown by it, and in each row a new cell with its
//! neighbour's properties (no span; a vertical merge carried on) and an empty paragraph --
//! beside a cell that spans the boundary (which keeps its span), else at the boundary; a
//! row skipping the grid there skips one column more. Tracked: each new paragraph's mark
//! inserted, and where a span or a skip was crossed, the old grid and rows recorded.
//!
//! Must not: decide what a tool's arguments mean (positions as accepted are the tool's),
//! or read past the table but for its ids.

use super::columns::skip;
use super::new_row::{clean_cell_properties, empty_paragraph_like};
use super::rows::{put, under_policy, Held};
use super::shape::{own_rows, row_cells, skips, Placed};
use super::tracker::TableTracker;
use super::TableChanged;
use crate::marks::mark;
use crate::names::{W, W14};
use crate::policy::RevisionPolicy;
use crate::review::xml::{digits, find_path, int, make, set_attr};
use crate::sink::EditSink;
use crate::stamping::w14_prefix;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_xml::QName;
use std::sync::Arc;

/// Where a row gets the new column.
enum Place {
    /// It skips the grid there: `w:gridBefore` (else `w:gridAfter`) one more.
    Skip(bool),
    /// A new cell like `neighbour`, after `anchor` (else before it).
    Cell { neighbour: Id, anchor: Id, after: bool },
}

/// Where the new column goes in `cells` (a row's, after `before` skipped columns and
/// before `after`), with `boundary` the grid line it goes at.
fn place(cells: &[Placed], (before, after): (usize, usize), index: usize, boundary: usize, right: bool) -> Place {
    let Some(last) = cells.last() else {
        return Place::Skip(true);
    };
    if before > 0 && boundary <= before {
        return Place::Skip(true);
    }
    let end = last.start + last.span;
    if boundary > end || (boundary == end && after > 0) {
        return Place::Skip(false);
    }
    if let Some(crossing) = cells.iter().find(|p| p.start < boundary && boundary < p.start + p.span) {
        return Place::Cell {
            neighbour: crossing.cell,
            anchor: crossing.cell,
            after: right,
        };
    }
    let neighbour = cells
        .iter()
        .find(|p| p.start <= index && index < p.start + p.span)
        .map_or(last.cell, |p| p.cell);
    match cells.iter().find(|p| p.start == boundary) {
        Some(at) => Place::Cell {
            neighbour,
            anchor: at.cell,
            after: false,
        },
        None => Place::Cell {
            neighbour,
            anchor: last.cell,
            after: true,
        },
    }
}

/// The grid column `index`'s `w:gridCol`, and the grid's.
fn grid_column(work: &Work, table: Id, index: usize) -> Option<Id> {
    let grid = work.find(table, W, "tblGrid")?;
    work.children(grid)
        .iter()
        .copied()
        .filter(|c| work.is(*c, W, "gridCol"))
        .nth(index)
}

/// A new cell like `neighbour` (no span, its width the column's), with an empty paragraph
/// like its first; the cell and the paragraph.
fn new_cell(work: &mut Work, neighbour: Id, width: Option<&str>) -> (Id, Id) {
    let cell = make(work, "tc", &[]);
    let found = work.find(neighbour, W, "tcPr");
    let properties = clean_cell_properties(work, found);
    if let Some(span) = work.find(properties, W, "gridSpan") {
        work.remove(span);
    }
    if let (Some(measure), Some(width)) = (work.find(properties, W, "tcW"), width) {
        if work.get(measure, Some(W), "type").is_none_or(|t| t == "dxa") {
            set_attr(work, measure, "w", width);
        }
    }
    work.append(cell, properties);
    let first = work.find(neighbour, W, "p");
    let paragraph = empty_paragraph_like(work, first);
    work.append(cell, paragraph);
    (cell, paragraph)
}

/// `insert_column`: a new column beside grid column `index`, to its right unless `right`
/// is false.
pub(super) fn insert_column(
    sink: &mut EditSink<'_, '_>,
    held: &mut Held,
    index: usize,
    right: bool,
    orders: &Orders,
) -> Result<TableChanged> {
    let work = &mut held.work;
    let table = work.root();
    let boundary = index + usize::from(right);
    let column = grid_column(work, table, index).ok_or_else(|| EditError::Path(held.found.part.clone()))?;
    let width = work.get(column, Some(W), "w").map(str::to_owned);
    let added = make(work, "gridCol", &[]);
    if let Some(width) = &width {
        set_attr(work, added, "w", width);
    }
    let whole = digits(work, Some(column), "w");
    if let Some(measure) = find_path(work, table, &["tblPr", "tblW"]) {
        if let (Some("dxa"), Some(whole)) = (work.get(measure, Some(W), "type"), whole) {
            let now = int(work, Some(measure), "w", 0) + whole;
            set_attr(work, measure, "w", &now.to_string());
        }
    }
    let rows = own_rows(work, table);
    let structural = rows.iter().any(|row| {
        skips(work, *row) != (0, 0)
            || row_cells(work, *row)
                .iter()
                .any(|p| p.start < boundary && boundary < p.start + p.span)
    });
    // the grid as it was, and the rows' skips, so a review gives them back whole
    let tracker = (matches!(sink.policy(), RevisionPolicy::Tracked(_)) && structural)
        .then(|| TableTracker::new(work, table, true));
    if right {
        work.add_next(column, added);
    } else {
        work.add_previous(column, added);
    }
    let paragraphs = add_cells(work, index, right, (width.as_deref(), whole), orders);
    let raws = stamp_new(sink, held, &paragraphs, "\0table")?;
    let tracked = under_policy(sink, |stamp| {
        if let Some(stamp) = stamp {
            for paragraph in &paragraphs {
                mark(&mut held.work, *paragraph, "ins", stamp, orders);
            }
            if let Some(tracker) = &tracker {
                tracker.record(&mut held.work, stamp, false, true, orders);
            }
        }
        Ok(())
    })?;
    put(sink, held, tracked)?;
    sink.ensure_w14(&held.found.part, false, &[])?;
    let created = created(sink, held, &raws)?;
    Ok(TableChanged {
        id: Some(held.id.clone()),
        created,
        ..TableChanged::default()
    })
}

/// The ids of the table's paragraphs whose paraId is one of `raws`, in document order.
fn created(sink: &EditSink<'_, '_>, held: &Held, raws: &[String]) -> Result<Vec<String>> {
    Ok(sink
        .docx()?
        .part(&held.found.part)
        .map(|part| {
            part.ids
                .paragraphs_within(&held.found.path)
                .into_iter()
                .filter(|p| p.raw.as_ref().is_some_and(|raw| raws.contains(raw)))
                .map(|p| p.id)
                .collect()
        })
        .unwrap_or_default())
}

/// In each row of the table, a new cell beside grid column `index` (to its right unless
/// `right` is false), or one more column skipped; `width` the column's as written and as
/// a whole number. The new cells' paragraphs.
fn add_cells(
    work: &mut Work,
    index: usize,
    right: bool,
    width: (Option<&str>, Option<i64>),
    orders: &Orders,
) -> Vec<Id> {
    let table = work.root();
    let boundary = index + usize::from(right);
    let (width, whole) = width;
    let mut paragraphs = Vec::new();
    for row in own_rows(work, table) {
        let cells = row_cells(work, row);
        match place(&cells, skips(work, row), index, boundary, right) {
            Place::Skip(before) => {
                let tags = if before {
                    ("gridBefore", "wBefore")
                } else {
                    ("gridAfter", "wAfter")
                };
                skip(work, row, tags, 1, whole, orders);
            }
            Place::Cell {
                neighbour,
                anchor,
                after,
            } => {
                let (cell, paragraph) = new_cell(work, neighbour, width);
                if after {
                    work.add_next(anchor, cell);
                } else {
                    work.add_previous(anchor, cell);
                }
                paragraphs.push(paragraph);
            }
        }
    }
    paragraphs
}

/// New paragraphs or rows given fresh `w14:paraId`/`w14:textId` values, drawn from the
/// part's sequence named by `suffix` (`_stamp_new`: `"\0table"`); the paraIds drawn.
pub(super) fn stamp_new(
    sink: &mut EditSink<'_, '_>,
    held: &mut Held,
    nodes: &[Id],
    suffix: &str,
) -> Result<Vec<String>> {
    let seed = format!("{}{suffix}", held.found.part);
    let drawn = sink.draw(&seed, 2 * nodes.len())?;
    let prefix = w14_prefix(&held.root, &held.found.path);
    let mut raws = Vec::new();
    for (node, pair) in nodes.iter().zip(drawn.chunks(2)) {
        for (local, value) in ["paraId", "textId"].into_iter().zip(pair) {
            held.work
                .set(*node, Arc::new(QName::new(Some(&prefix), local, Some(W14))), value);
        }
        raws.extend(pair.first().cloned());
    }
    Ok(raws)
}
