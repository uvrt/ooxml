//! Owns: a row copied with its content as docx-agent's `edit/tables.py` `copy_row` copies
//! it (Word's copy and paste of a row), untracked and tracked: the row's current view
//! (insertions kept, deletions and change records dropped) without its bookmarks,
//! permission ranges and comments (they stay with the row copied), a cell merged down
//! joining its merge where the copy lands inside it (`_join_merges`), placed beside the row,
//! every paragraph, row and drawing id fresh, and a drawing's number and a content
//! control's id as docx-agent's importer gives them ([`crate::imported`]); tracked, the copy
//! one insertion (the row's, every paragraph mark's and run's).
//!
//! Must not: decide what a tool's arguments mean, or read past the table but for its ids
//! (and, for a row holding a content control, the ids of the document's controls).

use super::new_column::stamp_new;
use super::new_row::{merges_note, new_row_paragraph, vmerge};
use super::rows::{put, under_policy, Held};
use super::shape::{own_rows, row_cells};
use super::TableChanged;
use crate::markdown::insert_content;
use crate::marks::{insert_in_order, mark};
use crate::names::{W, WP, WP14};
use crate::stamping::wp14_prefix;
use ooxml_xml::QName;
use std::sync::Arc;
use crate::review::xml::{find_path, make};
use crate::sink::EditSink;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};

/// Why a row is not copied (`_uncopyable`).
pub(super) fn check_copy(work: &Work, id: &str, rows: &[Id], index: usize) -> Result<()> {
    let Some(row) = rows.get(index).copied() else {
        return Err(EditError::Refused(format!(
            "{id} has no row {index} (rows 0-{})",
            rows.len().saturating_sub(1)
        )));
    };
    let problem = if !work.iter(row, W, &["footnoteReference", "endnoteReference"]).is_empty() {
        Some("holds a footnote or endnote reference (a note is not copied with a row)")
    } else if find_path(work, row, &["trPr", "del"]).is_some() {
        Some("is a pending tracked deletion (accept or reject it first)")
    } else if !work.iter(row, W, &["cellIns", "cellDel", "cellMerge"]).is_empty() {
        Some("holds tracked cell changes (accept or reject them first, word_changes within the row's table)")
    } else {
        None
    };
    if let Some(problem) = problem {
        return Err(EditError::Refused(format!(
            "row {index} of {id} {problem}: insert_row beside it, then set_cells the values"
        )));
    }
    Ok(())
}

/// The source's current view of a copied block (`_current_view`): insertions kept,
/// deletions dropped, records of changes dropped.
fn current_view(work: &mut Work, element: Id) {
    for node in work.iter(element, W, &["del", "moveFrom"]) {
        let Some(parent) = work.parent(node) else { continue };
        if work.is(parent, W, "trPr") && work.is(node, W, "del") {
            if let Some(row) = work.parent(parent).filter(|r| work.parent(*r).is_some()) {
                work.remove(row);
                continue;
            }
        }
        work.remove(node);
    }
    for node in work.iter(element, W, &["ins", "moveTo"]) {
        let Some(parent) = work.parent(node) else { continue };
        if work.is_any(parent, W, &["rPr", "trPr", "tcPr"]) {
            work.remove(node);
            continue;
        }
        for child in work.children(node).to_vec() {
            work.add_previous(node, child);
        }
        work.remove(node);
    }
    let records = [
        "rPrChange",
        "pPrChange",
        "sectPrChange",
        "tblPrChange",
        "trPrChange",
        "tcPrChange",
        "tblGridChange",
        "numberingChange",
        "cellIns",
        "cellDel",
        "cellMerge",
        "moveFromRangeStart",
        "moveFromRangeEnd",
        "moveToRangeStart",
        "moveToRangeEnd",
    ];
    for local in records {
        for node in work.iter(element, W, &[local]) {
            if work.parent(node).is_some() {
                work.remove(node);
            }
        }
    }
}

/// `node`'s run, or `node` alone where its run holds more (`_drop_run_of`).
pub(super) fn drop_run_of(work: &mut Work, node: Id) {
    let run = work.parent(node).filter(|r| work.is(*r, W, "r"));
    let alone = run.is_some_and(|r| {
        work.children(r)
            .iter()
            .all(|c| !work.is_element(*c) || *c == node || work.is(*c, W, "rPr"))
    });
    match run {
        Some(run) if alone => work.remove(run),
        _ => work.remove(node),
    }
}

/// The copy's bookmarks, permission ranges and comments left with the row copied
/// (`_row_copy`); what was left out, said.
fn leave_marks(work: &mut Work, row: Id) -> Vec<String> {
    let mut dropped = Vec::new();
    let marks = work.iter(row, W, &["bookmarkStart"]).len();
    if marks > 0 {
        dropped.push(format!("{marks} bookmark(s)"));
    }
    for node in work.iter(row, W, &["bookmarkStart", "bookmarkEnd", "permStart", "permEnd"]) {
        work.remove(node);
    }
    let mut comments: Vec<Option<String>> = Vec::new();
    for node in work.iter(row, W, &["commentReference", "commentRangeStart"]) {
        let id = work.get(node, Some(W), "id").map(str::to_owned);
        if !comments.contains(&id) {
            comments.push(id);
        }
    }
    if !comments.is_empty() {
        dropped.push(format!("{} comment(s)", comments.len()));
    }
    for node in work.iter(row, W, &["commentRangeStart", "commentRangeEnd", "commentReference"]) {
        if work.is(node, W, "commentReference") {
            drop_run_of(work, node);
        } else {
            work.remove(node);
        }
    }
    dropped
}

/// A copied row's cells merged down settled where the copy lands, `after` the row below
/// it (`_join_merges`): inside a merge (the row below continues it over the same columns)
/// a cell joins it, empty; elsewhere it is its own cell, with its text.
fn join_merges(work: &mut Work, row: Id, after: Option<Id>, orders: &Orders) {
    let below = after.map(|r| row_cells(work, r)).unwrap_or_default();
    for placed in row_cells(work, row) {
        let Some(merge) = find_path(work, placed.cell, &["tcPr", "vMerge"]) else {
            continue;
        };
        let other = below.iter().find(|b| b.start == placed.start && b.span == placed.span);
        if other.is_none_or(|o| vmerge(work, o.cell).as_deref() != Some("continue")) {
            work.remove(merge);
            continue;
        }
        work.remove_attr(merge, Some(W), "val");
        let paragraph = new_row_paragraph(work, placed.cell, orders);
        for child in work.children(placed.cell).to_vec() {
            if work.is_element(child) && !work.is(child, W, "tcPr") {
                work.remove(child);
            }
        }
        work.append(placed.cell, paragraph);
    }
}

/// A copied drawing's `wp14:anchorId` and `wp14:editId`, fresh (`_stamp`'s
/// `part + "\0drawing"` sequence).
fn stamp_drawing(sink: &mut EditSink<'_, '_>, held: &mut Held, node: Id) -> Result<()> {
    let seed = format!("{}\0drawing", held.found.part);
    let drawn = sink.draw(&seed, 2)?;
    let prefix = wp14_prefix(&held.root, &held.found.path);
    for (local, value) in ["anchorId", "editId"].into_iter().zip(&drawn) {
        held.work
            .set(node, Arc::new(QName::new(Some(&prefix), local, Some(WP14))), value);
    }
    Ok(())
}

/// The copy's ids, fresh (`_stamp`): the row's own and every nested row's under one seed,
/// its paragraphs' under another, its drawings' anchor and edit ids under a third; the
/// row's paraId and whether it holds a drawing.
fn stamp_copy(sink: &mut EditSink<'_, '_>, held: &mut Held, row: Id) -> Result<(String, bool)> {
    let mut drawn = String::new();
    let mut drawings = false;
    for node in held.work.descendants(row) {
        if held.work.is_any(node, WP, &["inline", "anchor"]) {
            stamp_drawing(sink, held, node)?;
            drawings = true;
            continue;
        }
        let seed = if held.work.is(node, W, "tr") {
            "\0row"
        } else if held.work.is(node, W, "p") {
            ""
        } else {
            continue;
        };
        let raws = stamp_new(sink, held, &[node], seed)?;
        if node == row {
            drawn = raws.into_iter().next().unwrap_or_default();
        }
    }
    Ok((drawn, drawings))
}

/// `copy_row`: a copy of row `index` with its content, below it unless `below` is false.
pub(super) fn copy_row(
    sink: &mut EditSink<'_, '_>,
    held: &mut Held,
    index: usize,
    below: bool,
    orders: &Orders,
) -> Result<TableChanged> {
    let work = &mut held.work;
    let table = work.root();
    let rows = own_rows(work, table);
    let source = *rows
        .get(index)
        .ok_or_else(|| EditError::Path(held.found.part.clone()))?;
    let row = work.deep_copy(source);
    current_view(work, row);
    let dropped = leave_marks(work, row);
    let after = if below {
        rows.get(index + 1).copied()
    } else {
        Some(source)
    };
    join_merges(work, row, after, orders);
    crate::imported::renumber_drawings(sink, work, row)?;
    crate::imported::renumber_controls(sink, work, row)?;
    if below {
        work.add_next(source, row);
    } else {
        work.add_previous(source, row);
    }
    let (drawn, drawings) = stamp_copy(sink, held, row)?;
    let tracked = under_policy(sink, |stamp| {
        let Some(stamp) = stamp else { return Ok(()) };
        let work = &mut held.work;
        let properties = match work.find(row, W, "trPr") {
            Some(p) => p,
            None => {
                let p = make(work, "trPr", &[]);
                insert_in_order(work, row, p, orders);
                p
            }
        };
        let record = stamp.make(work, "ins");
        insert_in_order(work, properties, record, orders);
        for paragraph in work.iter(row, W, &["p"]) {
            insert_content(work, paragraph, stamp);
            mark(work, paragraph, "ins", stamp, orders);
        }
        Ok(())
    })?;
    let own = own_rows(&held.work, table);
    let beside = own
        .iter()
        .position(|r| *r == row)
        .and_then(|at| merges_note(&held.work, &own, at));
    put(sink, held, tracked)?;
    sink.ensure_w14(&held.found.part, drawings, &[])?;
    let story = if held.story == "body" {
        String::new()
    } else {
        format!("{}/", held.story)
    };
    let new_id = format!("{story}tr:{drawn}");
    let mut warnings = Vec::new();
    if !dropped.is_empty() {
        warnings.push(format!("left on row {index}, not copied: {}", dropped.join(", ")));
    }
    warnings.extend(beside);
    Ok(TableChanged {
        id: Some(new_id.clone()),
        created: vec![new_id],
        warnings,
        ..TableChanged::default()
    })
}
