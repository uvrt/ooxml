//! Owns: a table's grid column deleted as docx-agent's `edit/tables.py` `delete_column`
//! does it, untracked and tracked: the cells only in the column go (tracked: `w:cellDel`
//! with their content and paragraph marks deleted), a cell spanning it spans one column
//! fewer and a row skipping it skips one fewer (tracked: their old properties and the old
//! grid recorded), the grid column goes, and, untracked, the table's width shrinks by the
//! column's and a row left without cells goes. A column wholly this author's pending
//! insertion goes outright, as its own pending rows do. Over pending tracked structure
//! (round 55, R115): a cell whose deletion is pending is left as it is, a cell a pending
//! merge took in gives its text back (its `w:cellDel` kept), and a column holding a
//! merge's first cell is refused.
//!
//! Must not: decide what a tool's arguments mean (positions as accepted are the tool's),
//! or read past the table but for its ids and its part's relationships.

use super::pending::{column_deleted, drop_copies, merge_origins, merged_away, no_reshape_over_merge, Reshape};
use super::rows::{put, under_policy, Held};
use super::shape::{grid_widths, own_rows, row_cells, set_grid};
use super::tracker::TableTracker;
use super::TableChanged;
use crate::across::rehome_markers;
use crate::inline::relationship_ids;
use crate::marks::{insert_in_order, mark, mark_record};
use crate::names::W;
use crate::policy::{RevisionPolicy, Stamp};
use crate::review::xml::{find_path, int, make, set_attr};
use crate::sink::EditSink;
use crate::table_delete::delete_content;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::Docx;

/// A note or comment reference.
const REFERENCES: [&str; 3] = ["footnoteReference", "endnoteReference", "commentReference"];

/// What deleting grid column `index` does to each row.
struct Plan {
    /// Cells only in the column.
    doomed: Vec<Id>,
    /// Cells spanning it and others.
    narrowed: Vec<Id>,
    /// Rows skipping it: before (`w:gridBefore`) or after (`w:gridAfter`).
    skipped: Vec<(Id, bool)>,
}

fn plan(work: &Work, table: Id, index: usize) -> Plan {
    let mut out = Plan {
        doomed: Vec::new(),
        narrowed: Vec::new(),
        skipped: Vec::new(),
    };
    for row in own_rows(work, table) {
        let before = usize::try_from(int(work, find_path(work, row, &["trPr", "gridBefore"]), "val", 0)).unwrap_or(0);
        match row_cells(work, row)
            .into_iter()
            .find(|p| p.start <= index && index < p.start + p.span)
        {
            None => out.skipped.push((row, index < before)),
            Some(p) if p.span == 1 => out.doomed.push(p.cell),
            Some(p) => out.narrowed.push(p.cell),
        }
    }
    out
}

/// The column's arguments against the table (`delete_column`'s refusals, before it edits);
/// `tracked`, a column holding a pending tracked merge's first cell too.
pub(super) fn check_column(work: &Work, id: &str, index: usize, tracked: bool) -> Result<()> {
    let table = work.root();
    let columns = grid_widths(work, table).len();
    if index >= columns {
        return Err(EditError::Refused(format!("{id} has no column {index}")));
    }
    if columns == 1 {
        return Err(EditError::Refused(format!(
            "{id} has one column; delete the table instead"
        )));
    }
    let rows = own_rows(work, table);
    let cells: Vec<Vec<super::shape::Placed>> = rows.iter().map(|r| row_cells(work, *r)).collect();
    let only = |p: &super::shape::Placed| p.start == index && p.span == 1;
    if cells
        .iter()
        .flatten()
        .any(|p| only(p) && !work.iter(p.cell, W, &REFERENCES).is_empty())
    {
        return Err(EditError::Unsupported(
            "word_edit_table delete_column of a column holding a note or comment reference".to_owned(),
        ));
    }
    if tracked {
        no_reshape_over_merge(work, Reshape::DeleteColumn(index), &format!("column {index}"))?;
    }
    let doomed = cells
        .iter()
        .flatten()
        .filter(|p| p.start <= index && index < p.start + p.span && p.span == 1)
        .count();
    let emptied = cells.iter().filter(|c| !c.is_empty()).all(|c| {
        c.iter()
            .all(|p| p.start <= index && index < p.start + p.span && p.span == 1)
    });
    if doomed > 0 && emptied {
        return Err(EditError::Refused(format!(
            "{id} would have no cells left; delete the table instead"
        )));
    }
    Ok(())
}

/// `delete_column`: grid column `index` deleted.
pub(super) fn delete_column(
    sink: &mut EditSink<'_, '_>,
    held: &mut Held,
    docx: &Docx,
    index: usize,
    orders: &Orders,
) -> Result<TableChanged> {
    let table = held.work.root();
    let widths = grid_widths(&held.work, table);
    let width = widths.get(index).copied().unwrap_or(0);
    let plan = plan(&held.work, table, index);
    let spans = !plan.narrowed.is_empty() || !plan.skipped.is_empty();
    let author = match sink.policy() {
        RevisionPolicy::Tracked(tracking) => Some(tracking.author.clone()),
        RevisionPolicy::Untracked => None,
    };
    // a column wholly this author's pending insertion goes outright (round 57)
    let own_column = author.as_deref().is_some_and(|author| {
        !plan.doomed.is_empty() && !spans && plan.doomed.iter().all(|c| own_inserted_cell(&held.work, *c, author))
    });
    let removed = if author.is_none() || own_column {
        cell_paragraphs(docx, held, &plan.doomed)
    } else {
        Vec::new()
    };
    let mut released = Vec::new();
    let mut warning = None;
    let tracked = under_policy(sink, |stamp| {
        let work = &mut held.work;
        match stamp {
            Some(stamp) if !own_column => {
                let tracker = spans.then(|| TableTracker::new(work, table, true));
                let origins = merge_origins(work, table);
                for cell in &plan.doomed {
                    if column_deleted(work, *cell) {
                        continue;
                    }
                    if merged_away(work, *cell) {
                        // a cell a pending tracked merge took in: its text leaves the merged
                        // cell again, as had the column gone first; its w:cellDel stays, now
                        // with its content deleted (round 55, R115)
                        drop_copies(work, origins.get(cell).copied(), *cell, stamp, orders);
                    } else {
                        record_deletion(work, *cell, stamp, orders);
                    }
                    released.extend(delete_cell(work, *cell, stamp, orders));
                }
                if let Some(tracker) = tracker {
                    reshape(work, table, &plan, &widths, index, orders);
                    tracker.record(work, stamp, false, true, orders);
                }
                if !plan.doomed.is_empty() {
                    warning = Some(format!(
                        "column {index} is deleted as a tracked change: word_read, render and word_edit_table's \
                         col positions count the table as accepted, without it; word_changes accept or reject \
                         within {} settles the deletion",
                        held.id
                    ));
                }
            }
            _ => {
                for cell in &plan.doomed {
                    rehome_markers(work, *cell);
                    released.extend(relationship_ids(work, *cell));
                    work.remove(*cell);
                }
                reshape(work, table, &plan, &widths, index, orders);
                if let Some(measure) = find_path(work, table, &["tblPr", "tblW"]) {
                    if work.get(measure, Some(W), "type") == Some("dxa") {
                        let now = (int(work, Some(measure), "w", 0) - width).max(0);
                        set_attr(work, measure, "w", &now.to_string());
                    }
                }
                for row in own_rows(work, table) {
                    if row_cells(work, row).is_empty() && work.parent(row).is_some() {
                        work.remove(row);
                    }
                }
            }
        }
        Ok(())
    })?;
    put(sink, held, tracked)?;
    sink.release(&held.found.part, &released)?;
    Ok(TableChanged {
        removed,
        warnings: warning.into_iter().collect(),
        ..TableChanged::default()
    })
}

/// The grid as it will be: a spanning cell one column narrower, a skipping row one fewer.
fn reshape(work: &mut Work, table: Id, plan: &Plan, widths: &[i64], index: usize, orders: &Orders) {
    let width = widths.get(index).copied().unwrap_or(0);
    for cell in &plan.narrowed {
        narrow(work, *cell, width);
    }
    for (row, before) in &plan.skipped {
        let (grid, measure) = if *before {
            ("gridBefore", "wBefore")
        } else {
            ("gridAfter", "wAfter")
        };
        skip(work, *row, (grid, measure), -1, (width >= 0).then_some(width), orders);
    }
    let mut left = widths.to_vec();
    if index < left.len() {
        left.remove(index);
    }
    set_grid(work, table, &left, orders);
}

/// A cell's deletion recorded (`w:cellDel`).
fn record_deletion(work: &mut Work, cell: Id, stamp: &mut Stamp, orders: &Orders) {
    let properties = match work.find(cell, W, "tcPr") {
        Some(p) => p,
        None => {
            let p = make(work, "tcPr", &[]);
            work.insert(cell, 0, p);
            p
        }
    };
    let record = stamp.make(work, "cellDel");
    insert_in_order(work, properties, record, orders);
}

/// A deleted cell's content and paragraph marks deleted, tracked; the relationship ids
/// released.
fn delete_cell(work: &mut Work, cell: Id, stamp: &mut Stamp, orders: &Orders) -> Vec<String> {
    let row_inserted = crate::review::xml::ancestor(work, cell, "tr")
        .and_then(|row| find_path(work, row, &["trPr", "ins"]))
        .is_some();
    let mut released = Vec::new();
    for paragraph in work.iter(cell, W, &["p"]) {
        released.extend(delete_content(work, paragraph, stamp));
        let mut record = mark_record(work, paragraph);
        if let Some(found) = record.filter(|r| row_inserted && work.is(*r, W, "ins")) {
            // a pending new row's cell: its mark deleted with the column (round 55, R115)
            let holder = work.parent(found);
            work.remove(found);
            if let Some(holder) = holder.filter(|h| work.children(*h).is_empty()) {
                work.remove(holder);
            }
            record = None;
        }
        let deleted_beside = |r: Id| work.parent(r).and_then(|h| work.find(h, W, "del")).is_some();
        if record.is_none_or(|r| work.is(r, W, "ins") && !deleted_beside(r)) {
            mark(work, paragraph, "del", stamp, orders);
        }
    }
    released
}

/// A cell spanning one grid column fewer, its width less `width` (`_narrow`).
fn narrow(work: &mut Work, cell: Id, width: i64) {
    let Some(properties) = work.find(cell, W, "tcPr") else {
        return;
    };
    let span = work.find(properties, W, "gridSpan");
    let value = int(work, span, "val", 1) - 1;
    if let Some(span) = span {
        if value > 1 {
            set_attr(work, span, "val", &value.to_string());
        } else {
            work.remove(span);
        }
    }
    if let Some(measure) = work.find(properties, W, "tcW") {
        let dxa = work.get(measure, Some(W), "type").is_none_or(|t| t == "dxa");
        let digits = crate::review::xml::digits(work, Some(measure), "w");
        if let (true, Some(now)) = (dxa, digits) {
            set_attr(work, measure, "w", &(now - width).max(0).to_string());
        }
    }
}

/// A row's `w:gridBefore`/`w:gridAfter` grown (or shrunk) by `delta` columns, its
/// `w:wBefore`/`w:wAfter` by as many columns' `width` where one is given (`_skip`).
pub(super) fn skip(
    work: &mut Work,
    row: Id,
    (grid, measure): (&str, &str),
    delta: i64,
    width: Option<i64>,
    orders: &Orders,
) {
    let properties = match work.find(row, W, "trPr") {
        Some(p) => p,
        None => {
            let p = make(work, "trPr", &[]);
            insert_in_order(work, row, p, orders);
            p
        }
    };
    let node = work.find(properties, W, grid);
    let value = (int(work, node, "val", 0) + delta).max(0);
    if let Some(node) = node {
        work.remove(node);
    }
    let measured = work.find(properties, W, measure);
    if value > 0 {
        let made = make(work, grid, &[("val", &value.to_string())]);
        insert_in_order(work, properties, made, orders);
        if let (Some(measured), Some(width)) = (measured, width) {
            if work.get(measured, Some(W), "type").is_none_or(|t| t == "dxa") {
                let now = (int(work, Some(measured), "w", 0) + delta * width).max(0);
                set_attr(work, measured, "w", &now.to_string());
            }
        }
    } else if let Some(measured) = measured {
        work.remove(measured);
    }
    if work.children(properties).is_empty() {
        work.remove(properties);
    }
}

/// Whether `cell` is wholly `author`'s pending insertion (`_own_inserted_cell`): every
/// paragraph mark inserted by them, every run inside their insertion, no nested table, no
/// cell record on it.
fn own_inserted_cell(work: &Work, cell: Id, author: &str) -> bool {
    if !work.iter(cell, W, &["tbl"]).is_empty() {
        return false;
    }
    if ["cellIns", "cellDel", "cellMerge"]
        .iter()
        .any(|local| find_path(work, cell, &["tcPr", local]).is_some())
    {
        return false;
    }
    let by_author = |r: Id| work.is(r, W, "ins") && work.get(r, Some(W), "author") == Some(author);
    let paragraphs = work.iter(cell, W, &["p"]);
    let own = paragraphs.iter().all(|p| {
        mark_record(work, *p).is_some_and(by_author)
            && find_path(work, *p, &["pPr", "rPr", "del"]).is_none()
            && work.iter(*p, W, &["r"]).into_iter().all(|run| {
                let mut at = work.parent(run);
                while let Some(node) = at.filter(|n| !work.is_any(*n, W, &["ins", "del", "p"])) {
                    at = work.parent(node);
                }
                at.is_some_and(by_author)
            })
    });
    own && !paragraphs.is_empty()
}

/// The ids (stamped) of the paragraphs in `cells`, in document order.
fn cell_paragraphs(docx: &Docx, held: &Held, cells: &[Id]) -> Vec<String> {
    let Some(part) = docx.part(&held.found.part) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for cell in cells {
        let Some(path) = held.path_of(*cell) else { continue };
        out.extend(part.ids.paragraphs_within(&path).into_iter().map(|p| p.id));
    }
    out
}
