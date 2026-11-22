//! Owns: the row docx-agent's `insert_row` puts beside another (`_put_row_like`, measured on
//! Word's Insert Rows): its shape (the row's own, or with `plain` the nearest row without a
//! cell over several grid columns, else a cell per grid column), its properties without
//! revision records, each cell's properties with a vertical merge carried on where the row
//! after the new one continues it, and each cell's empty paragraph formatted as Word formats
//! it (`_new_row_paragraph`); and the note docx-agent gives where the new row sits beside a
//! merge down (`merges_note`).
//!
//! Must not: put the row in the table, give it ids, or record it as a tracked change (the
//! command does, in that order).

use super::pending::{pending_column, Following};
use super::shape::{grid_widths, own_rows, row_cells, Placed};
use crate::names::W;
use crate::review::xml::{find_path, insert_in_order, make};
use crate::validate::Orders;
use crate::work::{Id, Work};

/// A cell's revision records, left out of a new cell's properties (`_CELL_RECORDS`).
const CELL_RECORDS: [&str; 4] = ["cellIns", "cellDel", "cellMerge", "tcPrChange"];
/// A paragraph mark's revision records (`track.MARKS`).
const MARKS: [&str; 4] = ["ins", "del", "moveFrom", "moveTo"];

/// A cell's vertical merge (`_vmerge`): `restart`, `continue` or none.
pub(crate) fn vmerge(work: &Work, cell: Id) -> Option<String> {
    let node = find_path(work, cell, &["tcPr", "vMerge"])?;
    Some(work.get(node, Some(W), "val").unwrap_or("continue").to_owned())
}

/// Whether a cell of `row` spans grid columns (`_spanned`).
fn spanned(work: &Work, row: Id) -> bool {
    row_cells(work, row).iter().any(|p| p.span > 1)
}

/// The row whose cells a new row beside `rows[at]` takes (`_shape_row`): the nearest row
/// with no cell over several columns, the one above first, a row whose deletion is pending
/// passed over.
fn shape_row(work: &Work, rows: &[Id], at: usize) -> Option<usize> {
    for distance in 1..rows.len() {
        let candidates = [at.checked_sub(distance), at.checked_add(distance)];
        for k in candidates.into_iter().flatten() {
            let Some(row) = rows.get(k).copied() else { continue };
            if !spanned(work, row)
                && !row_cells(work, row).is_empty()
                && find_path(work, row, &["trPr", "del"]).is_none()
            {
                return Some(k);
            }
        }
    }
    None
}

/// A cell's properties without revision records (`_clean_cell_properties`).
pub(super) fn clean_cell_properties(work: &mut Work, properties: Option<Id>) -> Id {
    let out = make(work, "tcPr", &[]);
    let Some(properties) = properties else { return out };
    for child in work.children(properties).to_vec() {
        if work.is_element(child) && !work.is_any(child, W, &CELL_RECORDS) {
            let copy = work.deep_copy(child);
            work.append(out, copy);
        }
    }
    out
}

/// A stand-in row with `reference`'s properties and a cell per grid column its cells cover
/// (`_row_per_column`).
fn row_per_column(work: &mut Work, table: Id, reference: Id, orders: &Orders) -> Id {
    let widths = grid_widths(work, table);
    let row = make(work, "tr", &[]);
    for local in ["tblPrEx", "trPr"] {
        if let Some(found) = work.find(reference, W, local) {
            let copy = work.deep_copy(found);
            work.append(row, copy);
        }
    }
    for placed in row_cells(work, reference) {
        for column in placed.start..placed.start + placed.span {
            let cell = work.deep_copy(placed.cell);
            for child in work.children(cell).to_vec() {
                if work.is_element(child) && !work.is(child, W, "tcPr") {
                    work.remove(child);
                }
            }
            let properties = match work.find(cell, W, "tcPr") {
                Some(p) => p,
                None => {
                    let p = make(work, "tcPr", &[]);
                    work.insert(cell, 0, p);
                    p
                }
            };
            set_span_width(work, properties, widths.get(column).copied(), orders);
            let first = work.find(placed.cell, W, "p");
            let paragraph = match first {
                Some(p) => work.deep_copy(p),
                None => make(work, "p", &[]),
            };
            work.append(cell, paragraph);
            work.append(row, cell);
        }
    }
    row
}

/// A cell's span set to one column of `width` twips (`_set_span_width` with a span of 1).
fn set_span_width(work: &mut Work, properties: Id, width: Option<i64>, orders: &Orders) {
    if let Some(span) = work.find(properties, W, "gridSpan") {
        work.remove(span);
    }
    let Some(width) = width else { return };
    let width = width.to_string();
    match work.find(properties, W, "tcW") {
        None => {
            let node = make(work, "tcW", &[("w", &width), ("type", "dxa")]);
            insert_in_order(work, properties, node, orders);
        }
        Some(node) if matches!(work.get(node, Some(W), "type"), None | Some("dxa" | "auto")) => {
            crate::review::xml::set_attr(work, node, "w", &width);
            crate::review::xml::set_attr(work, node, "type", "dxa");
        }
        Some(_) => {}
    }
}

/// `paragraph`'s first run that shows text (`first_text_run`): not one a deletion or move
/// takes away, nor one in a text box.
fn first_text_run(work: &Work, paragraph: Id) -> Option<Id> {
    work.iter(paragraph, W, &["r"]).into_iter().find(|run| {
        if work.find(*run, W, "t").is_none() {
            return false;
        }
        let mut holder = work.parent(*run);
        while let Some(node) = holder.filter(|n| *n != paragraph) {
            if work.is_any(node, W, &["moveFrom", "del", "p", "txbxContent"]) {
                return false;
            }
            holder = work.parent(node);
        }
        true
    })
}

/// A new empty paragraph with `paragraph`'s properties, no section break and no revision
/// record (`_empty_paragraph_like`).
pub(super) fn empty_paragraph_like(work: &mut Work, paragraph: Option<Id>) -> Id {
    let new = make(work, "p", &[]);
    let Some(properties) = paragraph.and_then(|p| work.find(p, W, "pPr")) else {
        return new;
    };
    let properties = work.deep_copy(properties);
    for child in work.children(properties).to_vec() {
        if work.is_any(child, W, &["sectPr", "pPrChange"]) {
            work.remove(child);
        } else if work.is(child, W, "rPr") {
            for mark in work.children(child).to_vec() {
                if work.is_any(mark, W, &MARKS) || work.is(mark, W, "rPrChange") {
                    work.remove(mark);
                }
            }
            if work.children(child).is_empty() {
                work.remove(child);
            }
        }
    }
    if !work.children(properties).is_empty() {
        work.append(new, properties);
    }
    new
}

/// The empty paragraph a new row's cell beside `cell` gets (`_new_row_paragraph`): the
/// cell's first paragraph's properties, its mark formatted as that paragraph's first run
/// of text where it has one.
pub(super) fn new_row_paragraph(work: &mut Work, cell: Id, orders: &Orders) -> Id {
    let source = work.find(cell, W, "p");
    let new = empty_paragraph_like(work, source);
    let Some(run) = source.and_then(|s| first_text_run(work, s)) else {
        return new;
    };
    let formatting = match work.find(run, W, "rPr") {
        Some(p) => work.deep_copy(p),
        None => make(work, "rPr", &[]),
    };
    for child in work.children(formatting).to_vec() {
        if work.is(child, W, "rPrChange") {
            work.remove(child);
        }
    }
    let properties = work.find(new, W, "pPr");
    if let Some(old) = properties.and_then(|p| work.find(p, W, "rPr")) {
        work.remove(old);
    }
    if work.children(formatting).is_empty() {
        if let Some(p) = properties.filter(|p| work.children(*p).is_empty()) {
            work.remove(p);
        }
        return new;
    }
    let properties = match properties {
        Some(p) => p,
        None => {
            let p = make(work, "pPr", &[]);
            work.insert(new, 0, p);
            p
        }
    };
    insert_in_order(work, properties, formatting, orders);
    new
}

/// Where the new row goes and what it is shaped like.
pub(crate) struct Beside {
    /// The row the new one goes beside.
    pub(crate) reference: Id,
    /// The row on the new one's other side, if any.
    pub(crate) neighbour: Option<Id>,
    /// Below `reference` (else above).
    pub(crate) below: bool,
    /// One cell per grid column beside a row with a cell over several.
    pub(crate) plain: bool,
}

/// The new row (not yet in the table) and what it says of its shape; with its cells in
/// columns whose tracked deletion or insertion is pending, for them to follow it
/// ([`super::pending::follow_pending_columns`]).
pub(crate) fn row_like(
    work: &mut Work,
    table: Id,
    beside: &Beside,
    orders: &Orders,
) -> (Id, Option<String>, Vec<Following>) {
    let reference = beside.reference;
    let mut shape = reference;
    let mut note = None;
    if beside.plain && spanned(work, reference) {
        let rows = own_rows(work, table);
        let at = rows.iter().position(|r| *r == reference).unwrap_or(0);
        if let Some(found) = shape_row(work, &rows, at) {
            shape = rows.get(found).copied().unwrap_or(reference);
            note = Some(format!(
                "row {at} has a cell over several columns: the new row has row {found}'s cells, one per column \
                 (merge cells for a span like row {at}'s)"
            ));
        } else {
            shape = row_per_column(work, table, reference, orders);
            note = Some(format!(
                "row {at} has a cell over several columns and every row has one: the new row has a cell for each \
                 of the table's {} grid columns",
                grid_widths(work, table).len()
            ));
        }
    }
    let row = make(work, "tr", &[]);
    if let Some(extra) = work.find(shape, W, "tblPrEx") {
        let copy = work.deep_copy(extra);
        work.append(row, copy);
    }
    if let Some(found) = work.find(shape, W, "trPr") {
        let properties = work.deep_copy(found);
        let header_kept = shape == reference || find_path(work, reference, &["trPr", "tblHeader"]).is_some();
        for child in work.children(properties).to_vec() {
            if work.is_any(child, W, &["ins", "del", "trPrChange"]) || (work.is(child, W, "tblHeader") && !header_kept)
            {
                work.remove(child);
            }
        }
        if !work.children(properties).is_empty() {
            work.append(row, properties);
        }
    }
    let after = if beside.below {
        beside.neighbour
    } else {
        Some(reference)
    };
    let after_cells: Vec<Placed> = after.map(|a| row_cells(work, a)).unwrap_or_default();
    let mut pending = Vec::new();
    for placed in row_cells(work, shape) {
        let cell = make(work, "tc", &[]);
        let own = work.find(placed.cell, W, "tcPr");
        let properties = clean_cell_properties(work, own);
        let merge = work.find(properties, W, "vMerge");
        if let Some(merge) = merge {
            work.remove(merge);
        }
        if merge.is_some() || shape != reference {
            let continued = after_cells
                .iter()
                .find(|a| a.start == placed.start && a.span == placed.span)
                .is_some_and(|a| vmerge(work, a.cell).as_deref() == Some("continue"));
            if continued {
                let node = make(work, "vMerge", &[]);
                insert_in_order(work, properties, node, orders);
            }
        }
        work.append(cell, properties);
        let paragraph = new_row_paragraph(work, placed.cell, orders);
        work.append(cell, paragraph);
        work.append(row, cell);
        let records = pending_column(work, placed.cell);
        if !records.is_empty() {
            pending.push(Following {
                cell,
                paragraph,
                records,
            });
        }
    }
    (row, note, pending)
}

/// The merges down that `rows[at]` sits right beside without joining (`merges_beside`):
/// `(grid column, the merge's first row, its last, "below" | "above")`.
fn merges_beside(work: &Work, rows: &[Id], at: usize) -> Vec<(usize, usize, usize, &'static str)> {
    let merge_at = |k: Option<usize>, start: usize, span: usize| -> Option<String> {
        let row = rows.get(k?)?;
        let cell = row_cells(work, *row)
            .into_iter()
            .find(|p| p.start == start && p.span == span)?;
        vmerge(work, cell.cell)
    };
    let merging = |m: Option<String>| matches!(m.as_deref(), Some("restart" | "continue"));
    let continues = |m: Option<String>| m.as_deref() == Some("continue");
    let mut out = Vec::new();
    let Some(row) = rows.get(at) else { return out };
    for placed in row_cells(work, *row) {
        if vmerge(work, placed.cell).is_some() {
            continue;
        }
        let (start, span) = (placed.start, placed.span);
        if merging(merge_at(at.checked_sub(1), start, span)) && !continues(merge_at(Some(at + 1), start, span)) {
            let mut first = at - 1;
            while first > 0 && continues(merge_at(Some(first), start, span)) {
                first -= 1;
            }
            if first + 1 < at {
                out.push((start, first, at - 1, "below"));
            }
        } else if merge_at(Some(at + 1), start, span).as_deref() == Some("restart")
            && continues(merge_at(Some(at + 2), start, span))
        {
            let mut last = at + 2;
            while continues(merge_at(Some(last + 1), start, span)) {
                last += 1;
            }
            out.push((start, at + 1, last, "above"));
        }
    }
    out
}

/// What [`merges_beside`] finds, said with the merge call that joins the row
/// (`merges_note`).
pub(crate) fn merges_note(work: &Work, rows: &[Id], at: usize) -> Option<String> {
    let found = merges_beside(work, rows, at);
    let (column, first, last, side) = *found.first()?;
    let corners = if side == "below" { (first, at) } else { (at, last) };
    let columns = found.iter().map(|f| f.0.to_string()).collect::<Vec<_>>().join(", ");
    Some(format!(
        "row {at} is {side} the merge down column {columns} (rows {first}-{last}), not in it: Word's Insert Rows puts \
         a row there outside the merge; a row added inside it (index {first}-{} after, or {}-{last} before) joins \
         it.  To join this one: word_edit_table merge first {{row: {}, col: {column}}} last {{row: {}, col: \
         {column}}}",
        last.saturating_sub(1),
        first + 1,
        corners.0,
        corners.1
    ))
}
