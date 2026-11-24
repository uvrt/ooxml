//! Owns: cells merged as docx-agent's `edit/tables.py` `merge_cells` merges them (Word's
//! Merge Cells), untracked and tracked: the rectangle grown to take in every cell it
//! touches whole (`_rectangle`), a merge already made answered as unchanged, and the merge
//! written -- untracked as Word writes it (`_merge_plain`: each row's first cell takes the
//! span and the vertical merge, the others' text goes to the first cell in reading order),
//! tracked in the form Word's Accept All and Reject All settle exactly (`_merge_tracked`:
//! the other cells `w:cellDel`, merged down as `w:cellMerge`, their text inserted at the
//! first cell's end).
//!
//! Tracked over pending tracked structure (round 55, R115, measured): a pending merge the
//! rectangle reaches taken in whole and counted as made, a row whose deletion is pending
//! and a cell a pending column deletion removes left out (`_tracked_plan`), a cell a
//! pending merge took in keeping its record (its text is in that merge's first cell).
//!
//! Must not: decide what a tool's arguments mean, or read past the table.

use super::new_column::stamp_new;
use super::new_row::{clean_cell_properties, vmerge};
use super::pending::{cell_merge, column_deleted, merged_away};
use super::rows::{put, under_policy, Held};
use super::shape::{own_rows, row_cells, Placed};
use super::TableChanged;
use crate::across::rehome_markers;
use crate::marks::{
    clean_properties, clear_revisions, insert_in_order, insert_paragraph_content, mark, mark_record,
    record_paragraph_change,
};
use crate::names::W;
use crate::policy::Stamp;
use crate::review::xml::{ancestor, digits, find_all, find_path, make, set_attr};
use crate::sink::EditSink;
use crate::table_delete::delete_content;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use std::collections::{HashMap, HashSet};

/// A grid position: (row, grid column).
pub(crate) type Position = (usize, usize);

/// A merge's rectangle, grown to take in whole cells, and each row's cells in it.
pub(super) struct Rectangle {
    pub(super) first: Position,
    pub(super) last: Position,
    pub(super) plan: Vec<Vec<Placed>>,
}

/// Each cell's rows: the first and last row of the vertical merge it is in, else its own
/// (docx-agent's `_layout`).
pub(super) fn extents(work: &Work, rows: &[Id]) -> HashMap<Id, (usize, usize)> {
    let mut out = HashMap::new();
    // grid column -> the cells (and rows) of the merge running down it
    let mut open: HashMap<usize, Vec<(Id, usize)>> = HashMap::new();
    for (r, row) in rows.iter().enumerate() {
        let cells = row_cells(work, *row);
        for placed in &cells {
            let merge = vmerge(work, placed.cell);
            match open.get_mut(&placed.start) {
                Some(group) if merge.as_deref() == Some("continue") => group.push((placed.cell, r)),
                _ => {
                    open.remove(&placed.start);
                    if matches!(merge.as_deref(), Some("restart" | "continue")) {
                        open.insert(placed.start, vec![(placed.cell, r)]);
                    } else {
                        out.insert(placed.cell, (r, r));
                    }
                }
            }
            if let Some(group) = open.get(&placed.start) {
                let first = group.first().map_or(r, |g| g.1);
                for (member, _) in group {
                    out.insert(*member, (first, r));
                }
            }
        }
        // a column no cell of this row continues ends its merge
        let starts: HashSet<usize> = cells.iter().map(|p| p.start).collect();
        open.retain(|column, _| starts.contains(column));
    }
    out
}

/// The rectangle `first`..`last` grown until no cell lies partly in it -- across a span,
/// down a vertical merge, and (`pending`: a tracked merge) over a pending tracked merge,
/// which counts as the one cell it makes -- and each row's cells in it (`_rectangle`).
pub(super) fn rectangle(work: &Work, first: Position, last: Position, pending: bool) -> Result<Rectangle> {
    let rows = own_rows(work, work.root());
    let ((mut r1, mut c1), (mut r2, mut c2)) = (first, last);
    if !(r1 <= r2 && r2 < rows.len()) || c1 > c2 {
        return Err(EditError::Refused(
            "first and last are (row, column), first above and left of last".to_owned(),
        ));
    }
    let layout = extents(work, &rows);
    let boxes = if pending {
        super::pending::pending_boxes(work, &rows)
    } else {
        Vec::new()
    };
    loop {
        let mut grown = false;
        for &((top, left), (bottom, right)) in &boxes {
            let meets = top <= r2 && bottom >= r1 && left <= c2 && right >= c1;
            let inside = r1 <= top && bottom <= r2 && c1 <= left && right <= c2;
            if meets && !inside {
                (r1, r2, c1, c2) = (r1.min(top), r2.max(bottom), c1.min(left), c2.max(right));
                grown = true;
            }
        }
        // the rows as they were when this pass began (Python's `range`)
        let (from, to) = (r1, r2);
        for r in from..=to {
            let Some(row) = rows.get(r) else { continue };
            for placed in row_cells(work, *row) {
                let end = placed.start + placed.span - 1;
                if end < c1 || placed.start > c2 {
                    continue;
                }
                if placed.start < c1 || end > c2 {
                    (c1, c2) = (c1.min(placed.start), c2.max(end));
                    grown = true;
                }
                let (top, bottom) = layout.get(&placed.cell).copied().unwrap_or((r, r));
                if top < r1 || bottom > r2 {
                    (r1, r2) = (r1.min(top), r2.max(bottom));
                    grown = true;
                }
            }
        }
        if !grown {
            break;
        }
    }
    let mut plan = Vec::new();
    for r in r1..=r2 {
        let inside: Vec<Placed> = rows
            .get(r)
            .map(|row| row_cells(work, *row))
            .unwrap_or_default()
            .into_iter()
            .filter(|p| c1 <= p.start && p.start <= c2)
            .collect();
        let fills =
            inside.first().is_some_and(|p| p.start == c1) && inside.last().is_some_and(|p| p.start + p.span - 1 == c2);
        if !fills {
            return Err(EditError::Refused(format!(
                "row {r} does not fill the cells from column {c1} to {c2}"
            )));
        }
        plan.push(inside);
    }
    Ok(Rectangle {
        first: (r1, c1),
        last: (r2, c2),
        plan,
    })
}

/// Whether the rectangle is one cell already: one row's one cell, or one cell per row
/// merged down over the same columns.
pub(super) fn merged_already(work: &Work, plan: &[Vec<Placed>]) -> bool {
    if !plan.iter().all(|row| row.len() == 1) {
        return false;
    }
    let leads: Vec<Placed> = plan.iter().filter_map(|row| row.first().copied()).collect();
    let Some((head, rest)) = leads.split_first() else {
        return true;
    };
    rest.is_empty()
        || (vmerge(work, head.cell).as_deref() == Some("restart")
            && rest.iter().all(|p| vmerge(work, p.cell).as_deref() == Some("continue"))
            && rest.iter().all(|p| (p.start, p.span) == (head.start, head.span)))
}

/// A tracked merge's rows and cells, what pending tracked changes take out left out
/// (`_tracked_plan`): a row whose deletion is pending (Word's Accept All removes it, the
/// merge continuing over it), a cell a pending column deletion removes -- a row or plan
/// that would be left empty kept whole.
pub(super) fn tracked_plan(work: &Work, plan: Vec<Vec<Placed>>) -> Vec<Vec<Placed>> {
    let plan = if plan.len() > 1 {
        let kept: Vec<Vec<Placed>> = plan
            .iter()
            .filter(|row| {
                row.first().is_some_and(|p| {
                    ancestor(work, p.cell, "tr").is_none_or(|r| find_path(work, r, &["trPr", "del"]).is_none())
                })
            })
            .cloned()
            .collect();
        if kept.is_empty() {
            plan
        } else {
            kept
        }
    } else {
        plan
    };
    plan.into_iter()
        .map(|row| {
            let kept: Vec<Placed> = row.iter().copied().filter(|p| !column_deleted(work, p.cell)).collect();
            if kept.is_empty() {
                row
            } else {
                kept
            }
        })
        .collect()
}

/// Whether the plan is one pending tracked merge already (`_merged_down_pending`): each
/// row's first cell joined by a `w:cellMerge` (`rest`, then `cont`) and the other cells
/// merged away.
pub(super) fn merged_down_pending(work: &Work, plan: &[Vec<Placed>]) -> bool {
    if !plan
        .iter()
        .all(|row| row.iter().skip(1).all(|p| merged_away(work, p.cell)))
    {
        return false;
    }
    let Some((head, rest)) = plan.split_first() else {
        return false;
    };
    if rest.is_empty() {
        return head.len() > 1;
    }
    let merge = |row: &Vec<Placed>| row.first().and_then(|p| cell_merge(work, p.cell));
    merge(head) == Some("rest") && rest.iter().all(|row| merge(row) == Some("cont"))
}

/// The warning where the rectangle grew.
pub(super) fn grown_note(rectangle: &Rectangle, first: Position, last: Position) -> Option<String> {
    let ((r1, c1), (r2, c2)) = (rectangle.first, rectangle.last);
    ((rectangle.first, rectangle.last) != (first, last))
        .then(|| format!("the merge takes in whole cells: ({r1}, {c1}) to ({r2}, {c2})"))
}

/// `merge_cells`: the rectangle `first`..`last` merged into one cell.
pub(super) fn merge_cells(
    sink: &mut EditSink<'_, '_>,
    held: &mut Held,
    first: Position,
    last: Position,
    orders: &Orders,
) -> Result<TableChanged> {
    let tracked = matches!(sink.policy(), crate::policy::RevisionPolicy::Tracked(_));
    let rectangle = rectangle(&held.work, first, last, tracked)?;
    let warnings: Vec<String> = grown_note(&rectangle, first, last).into_iter().collect();
    let plan = if tracked {
        tracked_plan(&held.work, rectangle.plan)
    } else {
        rectangle.plan
    };
    let origin = plan
        .first()
        .and_then(|row| row.first())
        .map(|p| p.cell)
        .ok_or_else(|| EditError::no_element(held.found.part.clone()))?;
    let (written, released, copied) = if tracked {
        let copies = tracked_copies(&mut held.work, &plan);
        stamp_new(sink, held, &copies, "\0table")?;
        let mut released = Vec::new();
        let written = under_policy(sink, |stamp| {
            if let Some(stamp) = stamp {
                released = merge_tracked(&mut held.work, &plan, origin, &copies, stamp, orders);
            }
            Ok(())
        })?;
        (written, released, !copies.is_empty())
    } else {
        let copies = merge_plain(&mut held.work, &plan, origin, orders);
        stamp_new(sink, held, &copies, "\0table")?;
        (None, Vec::new(), !copies.is_empty())
    };
    put(sink, held, written)?;
    sink.release(&held.found.part, &released)?;
    if copied {
        // the copies' ids declared (`_stamp_new`)
        sink.ensure_w14(&held.found.part, false, &[])?;
    }
    Ok(TableChanged {
        id: Some(held.id.clone()),
        warnings,
        ..TableChanged::default()
    })
}

/// Whether `paragraph` holds a run (`_has_text`).
pub(super) fn has_runs(work: &Work, paragraph: Id) -> bool {
    !work.iter(paragraph, W, &["r"]).is_empty()
}

/// A merged row's first cell's properties (`_merged_properties`): its own without records,
/// span or merge, spanning the row's cells, as wide as they are where each states a width
/// in twips.
fn merged_properties(work: &mut Work, row: &[Placed], orders: &Orders) -> Id {
    let lead = row.first().map(|p| p.cell);
    let found = lead.and_then(|l| work.find(l, W, "tcPr"));
    let properties = clean_cell_properties(work, found);
    for local in ["gridSpan", "vMerge", "hMerge"] {
        if let Some(node) = work.find(properties, W, local) {
            work.remove(node);
        }
    }
    let total: usize = row.iter().map(|p| p.span).sum();
    if total > 1 {
        let node = make(work, "gridSpan", &[("val", &total.to_string())]);
        insert_in_order(work, properties, node, orders);
    }
    if let Some(width) = work.find(properties, W, "tcW") {
        let widths: Option<Vec<i64>> = row
            .iter()
            .map(|p| {
                let measure = find_path(work, p.cell, &["tcPr", "tcW"])?;
                matches!(work.get(measure, Some(W), "type"), None | Some("dxa"))
                    .then(|| digits(work, Some(measure), "w"))
                    .flatten()
            })
            .collect();
        if let Some(widths) = widths {
            set_attr(work, width, "w", &widths.iter().sum::<i64>().to_string());
        }
    }
    properties
}

/// The paragraphs of `cell` (its own, not a nested table's).
fn paragraphs(work: &Work, cell: Id) -> Vec<Id> {
    find_all(work, cell, "p")
}

/// Untracked, as Word merges (`_merge_plain`): each row's first cell takes the span (and
/// the vertical merge); every other cell's paragraphs with text go to the end of the first
/// cell in reading order (copies, from a row's first cell below, which keeps its last
/// paragraph, emptied); the other cells go. The copies made.
fn merge_plain(work: &mut Work, plan: &[Vec<Placed>], origin: Id, orders: &Orders) -> Vec<Id> {
    let mut moved: Vec<(Id, bool)> = Vec::new();
    for (k, row) in plan.iter().enumerate() {
        let Some(lead) = row.first().map(|p| p.cell) else {
            continue;
        };
        let properties = merged_properties(work, row, orders);
        if plan.len() > 1 {
            let attributes: &[(&str, &str)] = if k == 0 { &[("val", "restart")] } else { &[] };
            let node = make(work, "vMerge", attributes);
            insert_in_order(work, properties, node, orders);
        }
        if let Some(old) = work.find(lead, W, "tcPr") {
            work.remove(old);
        }
        work.insert(lead, 0, properties);
        if k > 0 {
            let own = paragraphs(work, lead);
            for paragraph in &own {
                if has_runs(work, *paragraph) {
                    let copy = work.deep_copy(*paragraph);
                    clear_revisions(work, copy);
                    let markers = ["bookmarkStart", "bookmarkEnd", "commentRangeStart", "commentRangeEnd"];
                    for marker in work.iter(copy, W, &markers) {
                        work.remove(marker);
                    }
                    moved.push((copy, true));
                }
            }
            for table in find_all(work, lead, "tbl") {
                moved.push((table, false));
            }
            if let Some((last, rest)) = own.split_last() {
                for paragraph in rest {
                    rehome_markers(work, *paragraph);
                    work.remove(*paragraph);
                }
                for child in work.children(*last).to_vec() {
                    if !work.is_any(child, W, &["pPr", "bookmarkStart", "bookmarkEnd"]) {
                        work.remove(child);
                    }
                }
            }
        }
        for placed in row.iter().skip(1) {
            for block in work.children(placed.cell).to_vec() {
                if work.is(block, W, "tbl") || (work.is(block, W, "p") && has_runs(work, block)) {
                    moved.push((block, false));
                } else if work.is(block, W, "p") {
                    rehome_markers(work, block);
                }
            }
            work.remove(placed.cell);
        }
    }
    for (block, _) in &moved {
        work.append(origin, *block);
    }
    end_with_paragraph(work, origin);
    moved.into_iter().filter(|m| m.1).map(|m| m.0).collect()
}

/// A cell ends with a paragraph (`_end_with_paragraph`).
pub(super) fn end_with_paragraph(work: &mut Work, cell: Id) {
    let last = work
        .children(cell)
        .iter()
        .rev()
        .copied()
        .find(|b| work.is_any(*b, W, &["p", "tbl", "sdt"]));
    if last.is_none_or(|b| !work.is(b, W, "p")) {
        let paragraph = make(work, "p", &[]);
        work.append(cell, paragraph);
    }
}

/// The comments whose range and reference all lie in `paragraph` (`_whole_comments`).
fn whole_comments(work: &Work, paragraph: Id) -> Vec<String> {
    let ids = |local: &str| -> Vec<String> {
        work.iter(paragraph, W, &[local])
            .into_iter()
            .map(|n| work.get(n, Some(W), "id").unwrap_or_default().to_owned())
            .collect()
    };
    let (ends, references) = (ids("commentRangeEnd"), ids("commentReference"));
    ids("commentRangeStart")
        .into_iter()
        .filter(|i| ends.contains(i) && references.contains(i))
        .collect()
}

/// A tracked merge's copy holds the paragraph's comment and note references once
/// (`_move_references`): a comment wholly in it and a note's reference move to the copy; a
/// comment reaching outside stays, its reference left out of the copy.
fn move_references(work: &mut Work, original: Id, copy: Id) {
    let whole = whole_comments(work, original);
    let held = |work: &Work, node: Id| {
        work.get(node, Some(W), "id")
            .is_some_and(|i| whole.iter().any(|w| w == i))
    };
    for node in work.iter(copy, W, &["commentReference"]) {
        if !held(work, node) {
            super::copy_row::drop_run_of(work, node);
        }
    }
    let mut moved = work.iter(original, W, &["footnoteReference", "endnoteReference"]);
    moved.extend(
        work.iter(
            original,
            W,
            &["commentReference", "commentRangeStart", "commentRangeEnd"],
        )
        .into_iter()
        .filter(|n| held(work, *n)),
    );
    for node in moved {
        if work.is_any(node, W, &["commentRangeStart", "commentRangeEnd"]) {
            work.remove(node);
        } else {
            super::copy_row::drop_run_of(work, node);
        }
    }
}

/// A tracked merge's cells of one row: its first, and the others but those a pending
/// tracked merge already took in (they keep their record; their text is in that merge's
/// first cell, which this merge takes in).
fn taken_cells(work: &Work, row: &[Placed]) -> Vec<Id> {
    row.iter()
        .enumerate()
        .filter(|(k, p)| *k == 0 || !merged_away(work, p.cell))
        .map(|(_, p)| p.cell)
        .collect()
}

/// A tracked merge's copies of the other cells' paragraphs with text, in reading order
/// (`_merge_tracked`'s first part): plain, without bookmarks or comments reaching outside,
/// each paragraph's references moved to its copy.
fn tracked_copies(work: &mut Work, plan: &[Vec<Placed>]) -> Vec<Id> {
    let mut copies = Vec::new();
    for (k, row) in plan.iter().enumerate() {
        let cells: Vec<Id> = taken_cells(work, row).into_iter().skip(usize::from(k == 0)).collect();
        for cell in cells {
            for paragraph in paragraphs(work, cell) {
                if !has_runs(work, paragraph) {
                    continue;
                }
                let whole = whole_comments(work, paragraph);
                let copy = work.deep_copy(paragraph);
                clear_revisions(work, copy);
                let markers = ["bookmarkStart", "bookmarkEnd", "commentRangeStart", "commentRangeEnd"];
                for marker in work.iter(copy, W, &markers) {
                    let bookmark = work.is_any(marker, W, &["bookmarkStart", "bookmarkEnd"]);
                    let id = work.get(marker, Some(W), "id").unwrap_or_default();
                    if bookmark || !whole.iter().any(|w| w == id) {
                        work.remove(marker);
                    }
                }
                if has_runs(work, copy) {
                    move_references(work, paragraph, copy);
                    copies.push(copy);
                }
            }
        }
    }
    copies
}

/// Tracked (`_merge_tracked`, measured): merged down, each row's first cell a
/// `w:cellMerge` (`rest`, then `cont`; the old merge in `vMergeOrig`) and a continuing
/// row's first cell's content deleted; every other cell `w:cellDel`; the copies inserted at
/// the first cell's end. The relationship ids released.
fn merge_tracked(
    work: &mut Work,
    plan: &[Vec<Placed>],
    origin: Id,
    copies: &[Id],
    stamp: &mut Stamp,
    orders: &Orders,
) -> Vec<String> {
    let mut released = Vec::new();
    for (k, row) in plan.iter().enumerate() {
        let Some(lead) = row.first().map(|p| p.cell) else {
            continue;
        };
        if plan.len() > 1 {
            let properties = match work.find(lead, W, "tcPr") {
                Some(p) => p,
                None => {
                    let p = make(work, "tcPr", &[]);
                    work.insert(lead, 0, p);
                    p
                }
            };
            let now = if k == 0 { "rest" } else { "cont" };
            if let Some(record) = work.find(properties, W, "cellMerge") {
                // merged down already, pending: the record says where it is now
                set_attr(work, record, "vMerge", now);
            } else {
                let had = vmerge(work, lead);
                if let Some(existing) = work.find(properties, W, "vMerge") {
                    work.remove(existing);
                }
                let record = stamp.make(work, "cellMerge");
                set_attr(work, record, "vMerge", now);
                if let Some(had) = had {
                    set_attr(
                        work,
                        record,
                        "vMergeOrig",
                        if had == "restart" { "rest" } else { "cont" },
                    );
                }
                insert_in_order(work, properties, record, orders);
            }
        }
        if k > 0 {
            // a continuing row's first cell stays (merged into the one above): its content goes
            for paragraph in paragraphs(work, lead) {
                released.extend(delete_content(work, paragraph, stamp));
            }
        }
        for cell in taken_cells(work, row).into_iter().skip(1) {
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
        if k > 0 {
            // a continuation keeps its last paragraph: the others' marks go
            let own = paragraphs(work, lead);
            for paragraph in own.iter().take(own.len().saturating_sub(1)) {
                if mark_record(work, *paragraph).is_none() {
                    mark(work, *paragraph, "del", stamp, orders);
                }
            }
        }
    }
    if let Some(last_paragraph) = paragraphs(work, origin).last().copied() {
        append_copies(work, last_paragraph, copies, stamp, orders);
    }
    released
}

/// A tracked merge's copies inserted after the first cell's last paragraph, as Word writes
/// paragraphs typed there: each mark but the last inserted, the last carrying the old
/// mark's properties.
fn append_copies(work: &mut Work, last_paragraph: Id, copies: &[Id], stamp: &mut Stamp, orders: &Orders) {
    if copies.is_empty() {
        return;
    }
    for copy in copies {
        insert_paragraph_content(work, *copy, stamp);
    }
    let mut anchor = last_paragraph;
    for copy in copies {
        work.add_next(anchor, *copy);
        anchor = *copy;
    }
    let sequence: Vec<Id> = std::iter::once(last_paragraph).chain(copies.iter().copied()).collect();
    for paragraph in sequence.iter().take(sequence.len() - 1) {
        if mark_record(work, *paragraph).is_none() {
            mark(work, *paragraph, "ins", stamp, orders);
        }
    }
    let found = work.find(last_paragraph, W, "pPr");
    let old = clean_properties(work, found, last_paragraph);
    if let Some(final_copy) = copies.last() {
        record_paragraph_change(work, *final_copy, old, stamp, orders);
    }
}
