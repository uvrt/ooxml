//! Owns: a cell split as docx-agent's `edit/tables.py` `split_cell` splits it (Word's Split
//! Cells, measured), untracked and tracked: across (`_split_across`), a cell over enough
//! grid columns divided along the grid's lines, else its grid columns divided and every
//! other row's cell over them spanning the new ones; down (`_split_down`), a vertical merge
//! divided, or new rows below it with the row's other cells merged down into them. The
//! content stays in the first cell; the new cells hold an empty paragraph, stamped. Tracked,
//! the new rows and paragraph marks are insertions, the other cells' old properties and the
//! old grid recorded, a divided merge as `w:cellMerge`.
//!
//! Must not: decide what a tool's arguments mean, or read past the table but for its ids.

use super::columns::skip;
use super::merge::extents;
use super::new_column::stamp_new;
use super::new_row::{clean_cell_properties, empty_paragraph_like, vmerge};
use super::rows::{merges_as_cell_merges, put, under_policy, Held};
use super::shape::{grid_widths, own_rows, row_cells, set_grid};
use super::tracker::TableTracker;
use super::TableChanged;
use crate::marks::{insert_in_order, mark, mark_record};
use crate::names::W;
use crate::policy::RevisionPolicy;
use crate::review::xml::{ancestor, find_all, find_path, int, make, set_attr};
use crate::sink::EditSink;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use std::collections::HashMap;

/// A grid position: (row, grid column).
type Spot = (usize, usize);

/// Where Word's even division would have added grid columns: the cell's span and each new
/// cell's.
type Uneven = Option<(usize, Vec<usize>)>;

/// The cell split: its row, grid column, and into how many rows and columns.
#[derive(Debug, Clone, Copy)]
pub(super) struct Split {
    pub(super) row: usize,
    pub(super) column: usize,
    pub(super) rows: usize,
    pub(super) columns: usize,
}

/// A table this edit cannot place a line of its grid in (a row past its grid).
fn off_grid() -> EditError {
    EditError::Unsupported("word_edit_table split of a cell in a row reaching past the table's grid".to_owned())
}

/// The split's columns (the cell's span where it spans several, else 2, by default), its
/// arguments checked as `split_cell` checks them; `None` where it changes nothing (1 x 1).
pub(super) fn plan_split(
    work: &Work,
    id: &str,
    (row, column): (usize, usize),
    rows: usize,
    columns: Option<usize>,
) -> Result<Option<usize>> {
    let all = own_rows(work, work.root());
    let target = all.get(row).and_then(|r| {
        row_cells(work, *r)
            .into_iter()
            .find(|p| p.start <= column && column < p.start + p.span)
    });
    let columns = columns.unwrap_or_else(|| target.filter(|t| t.span > 1).map_or(2, |t| t.span));
    if !(1..=63).contains(&rows) || !(1..=63).contains(&columns) {
        return Err(EditError::Refused(
            "rows and columns are whole numbers from 1 to 63".to_owned(),
        ));
    }
    if rows == 1 && columns == 1 {
        return Ok(None);
    }
    if target.is_none() {
        return Err(EditError::Refused(no_cell_message(work, &all, id, row, column)));
    }
    Ok(Some(columns))
}

/// `t:X has no cell at (1, 2)`, then what the table has (docx-agent's `no_cell_message` over
/// `Table.grid`): its rows and columns, each merged cell with the positions it covers, the
/// positions no cell covers.
fn no_cell_message(work: &Work, rows: &[Id], id: &str, row: usize, column: usize) -> String {
    let mut grid: Vec<Vec<Option<Spot>>> = Vec::new();
    let mut origins: HashMap<usize, Spot> = HashMap::new();
    for (r, element) in rows.iter().enumerate() {
        let before =
            usize::try_from(int(work, find_path(work, *element, &["trPr", "gridBefore"]), "val", 0)).unwrap_or(0);
        let mut line = vec![None; before];
        for placed in row_cells(work, *element) {
            let continues = vmerge(work, placed.cell).as_deref() == Some("continue");
            let origin = continues
                .then(|| origins.get(&placed.start).copied())
                .flatten()
                .unwrap_or_else(|| {
                    for c in placed.start..placed.start + placed.span {
                        origins.insert(c, (r, placed.start));
                    }
                    (r, placed.start)
                });
            line.extend(std::iter::repeat_n(Some(origin), placed.span));
        }
        grid.push(line);
    }
    let columns = grid.iter().map(Vec::len).max().unwrap_or(0);
    let mut spans: Vec<(Spot, Vec<Spot>)> = Vec::new();
    let mut empty = Vec::new();
    for (r, line) in grid.iter().enumerate() {
        for c in 0..columns {
            match line.get(c).copied().flatten() {
                None => empty.push(format!("({r}, {c})")),
                Some(origin) => match spans.iter_mut().find(|(o, _)| *o == origin) {
                    Some((_, cells)) => cells.push((r, c)),
                    None => spans.push((origin, vec![(r, c)])),
                },
            }
        }
    }
    let merged: Vec<String> = spans
        .iter()
        .filter(|(_, cells)| cells.len() > 1)
        .filter_map(|(o, cells)| cells.last().map(|l| format!("({}, {})-({}, {})", o.0, o.1, l.0, l.1)))
        .collect();
    let count = |n: usize| i64::try_from(n).unwrap_or(i64::MAX) - 1;
    let mut text = format!(
        "{id} has no cell at ({row}, {column}): it has {} row(s) (0 to {}) and {columns} column(s) (0 to {}), \
         counted from 0",
        grid.len(),
        count(grid.len()),
        count(columns)
    );
    let listed = |items: &[String]| {
        let shown = items.iter().take(10).cloned().collect::<Vec<_>>().join(", ");
        if items.len() > 10 {
            format!("{shown} and {} more", items.len() - 10)
        } else {
            shown
        }
    };
    if !merged.is_empty() {
        text += &format!(
            "; merged: {} (a merged cell's id is its first position's)",
            listed(&merged)
        );
    }
    if !empty.is_empty() {
        text += &format!("; no cell at {}", listed(&empty));
    }
    text
}

/// A cell's span set to `span` grid columns and its width to `width` twips
/// (`_set_span_width`).
fn set_span_width(work: &mut Work, properties: Id, span: usize, width: Option<i64>, orders: &Orders) {
    if let Some(node) = work.find(properties, W, "gridSpan") {
        work.remove(node);
    }
    if span > 1 {
        let node = make(work, "gridSpan", &[("val", &span.to_string())]);
        insert_in_order(work, properties, node, orders);
    }
    let Some(width) = width else { return };
    let width = width.to_string();
    match work.find(properties, W, "tcW") {
        None => {
            let node = make(work, "tcW", &[("w", &width), ("type", "dxa")]);
            insert_in_order(work, properties, node, orders);
        }
        Some(node) if matches!(work.get(node, Some(W), "type"), None | Some("dxa" | "auto")) => {
            set_attr(work, node, "w", &width);
            set_attr(work, node, "type", "dxa");
        }
        Some(_) => {}
    }
}

/// A `w:vMerge` (`restart` written, `continue` as no value).
fn merge_node(work: &mut Work, merge: &str) -> Id {
    let attributes: &[(&str, &str)] = if merge == "restart" { &[("val", "restart")] } else { &[] };
    make(work, "vMerge", attributes)
}

/// A new cell with `cell`'s properties (no span, merge or records but those given),
/// holding an empty paragraph like its first (`_new_cell_like`).
fn new_cell_like(
    work: &mut Work,
    cell: Id,
    span: usize,
    width: Option<i64>,
    merge: Option<&str>,
    orders: &Orders,
) -> Id {
    let found = work.find(cell, W, "tcPr");
    let properties = clean_cell_properties(work, found);
    for local in ["gridSpan", "vMerge", "hMerge"] {
        if let Some(node) = work.find(properties, W, local) {
            work.remove(node);
        }
    }
    set_span_width(work, properties, span, width, orders);
    if let Some(merge) = merge {
        let node = merge_node(work, merge);
        insert_in_order(work, properties, node, orders);
    }
    let new = make(work, "tc", &[]);
    work.append(new, properties);
    let first = work.find(cell, W, "p");
    let paragraph = empty_paragraph_like(work, first);
    work.append(new, paragraph);
    new
}

/// A cell's vertical merge set (`_set_merge`); its properties dropped where left empty.
fn set_merge(work: &mut Work, cell: Id, merge: Option<&str>, orders: &Orders) {
    let properties = match (work.find(cell, W, "tcPr"), merge) {
        (Some(p), _) => p,
        (None, None) => return,
        (None, Some(_)) => {
            let p = make(work, "tcPr", &[]);
            work.insert(cell, 0, p);
            p
        }
    };
    if let Some(node) = work.find(properties, W, "vMerge") {
        work.remove(node);
    }
    if let Some(merge) = merge {
        let node = merge_node(work, merge);
        insert_in_order(work, properties, node, orders);
    }
    if work.children(properties).is_empty() {
        work.remove(properties);
    }
}

/// `cell`'s properties, made where it has none (put first).
fn properties_of(work: &mut Work, cell: Id) -> Id {
    if let Some(found) = work.find(cell, W, "tcPr") {
        return found;
    }
    let made = make(work, "tcPr", &[]);
    work.insert(cell, 0, made);
    made
}

/// The cell of `row` starting at grid column `start`.
fn cell_at(work: &Work, row: Option<&Id>, start: usize) -> Option<Id> {
    row_cells(work, *row?)
        .into_iter()
        .find(|p| p.start == start)
        .map(|p| p.cell)
}

/// Python's `round` of `a / b`: to the nearest, a half to the even.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn round_div(a: i64, b: i64) -> i64 {
    (a as f64 / b as f64).round_ties_even() as i64
}

/// Every row's cells and skips re-placed on a grid whose lines are now `new` (were `lines`).
fn regrid(work: &mut Work, lines: &[i64], new: &[i64], orders: &Orders) -> Result<()> {
    let table = work.root();
    let line = |k: usize| lines.get(k).copied().ok_or_else(off_grid);
    let mut positions = Vec::new();
    for row in own_rows(work, table) {
        let (before, after) = skips(work, row);
        let cells = row_cells(work, row);
        let placed = cells
            .iter()
            .map(|p| Ok((p.cell, line(p.start)?, line(p.start + p.span)?)))
            .collect::<Result<Vec<_>>>()?;
        let reach = before + cells.iter().map(|p| p.span).sum::<usize>() + after;
        let end = line(reach.min(lines.len().saturating_sub(1)))?;
        positions.push((row, line(before)?, placed, end));
    }
    let index: HashMap<i64, usize> = new.iter().enumerate().map(|(k, x)| (*x, k)).collect();
    let at = |x: &i64| i64::try_from(index.get(x).copied().unwrap_or(0)).unwrap_or(0);
    let widths: Vec<i64> = new
        .windows(2)
        .map(|w| w.get(1).copied().unwrap_or(0) - w.first().copied().unwrap_or(0))
        .collect();
    set_grid(work, table, &widths, orders);
    for (row, left, placed, right) in positions {
        let (before, after) = skips(work, row);
        if before > 0 {
            skip(
                work,
                row,
                ("gridBefore", "wBefore"),
                at(&left) - signed(before),
                None,
                orders,
            );
        }
        for (cell, a, b) in &placed {
            let properties = properties_of(work, *cell);
            if let Some(node) = work.find(properties, W, "gridSpan") {
                work.remove(node);
            }
            let span = at(b) - at(a);
            if span > 1 {
                let node = make(work, "gridSpan", &[("val", &span.to_string())]);
                insert_in_order(work, properties, node, orders);
            }
            if work.children(properties).is_empty() {
                work.remove(properties);
            }
        }
        if after > 0 {
            let last = placed.last().map_or(left, |p| p.2);
            skip(
                work,
                row,
                ("gridAfter", "wAfter"),
                (at(&right) - at(&last)) - signed(after),
                None,
                orders,
            );
        }
    }
    Ok(())
}

fn signed(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// A row's `w:gridBefore` and `w:gridAfter` (`_skips`).
fn skips(work: &Work, row: Id) -> (usize, usize) {
    let of = |local: &str| usize::try_from(int(work, find_path(work, row, &["trPr", local]), "val", 0)).unwrap_or(0);
    (of("gridBefore"), of("gridAfter"))
}

/// `cell` (and the cells under it in its vertical merge, rows `top`..=`bottom`) split into
/// `n` across, as Word does (`_split_across`). The new cells in document order; and where
/// Word's even division would have added grid columns, the cell's span and each new cell's.
fn split_across(
    work: &mut Work,
    rows: &[Id],
    (top, bottom): (usize, usize),
    cell: Id,
    n: usize,
    orders: &Orders,
) -> Result<(Vec<Id>, Uneven)> {
    let table = work.root();
    let mut lines = vec![0i64];
    for width in grid_widths(work, table) {
        lines.push(lines.last().copied().unwrap_or(0) + width);
    }
    let placed = rows
        .get(top)
        .and_then(|r| row_cells(work, *r).into_iter().find(|p| p.cell == cell))
        .ok_or_else(off_grid)?;
    let (start, span) = (placed.start, placed.span);
    let x0 = lines.get(start).copied().ok_or_else(off_grid)?;
    let x1 = lines.get(start + span).copied().ok_or_else(off_grid)?;
    let parts = signed(n);
    let mut cuts: Vec<i64> = (1..parts).map(|k| x0 + round_div((x1 - x0) * k, parts)).collect();
    let mut uneven = None;
    if 1 < n && n <= span {
        let sizes: Vec<usize> = (0..n).map(|k| span / n + usize::from(k < span % n)).collect();
        if !cuts.iter().all(|c| lines.contains(c)) {
            uneven = Some((span, sizes.clone()));
        }
        cuts = (1..n)
            .map(|k| {
                lines
                    .get(start + sizes.iter().take(k).sum::<usize>())
                    .copied()
                    .ok_or_else(off_grid)
            })
            .collect::<Result<_>>()?;
    }
    let mut new_lines: Vec<i64> = lines.iter().chain(cuts.iter()).copied().collect();
    new_lines.sort_unstable();
    new_lines.dedup();
    if new_lines != lines {
        regrid(work, &lines, &new_lines, orders)?;
        lines = new_lines;
    }
    let index: HashMap<i64, usize> = lines.iter().enumerate().map(|(k, x)| (*x, k)).collect();
    let at = |x: i64| index.get(&x).copied().unwrap_or(0);
    let edges: Vec<i64> = std::iter::once(x0)
        .chain(cuts.iter().copied())
        .chain(std::iter::once(x1))
        .collect();
    let piece = |k: usize| {
        let (a, b) = (
            edges.get(k).copied().unwrap_or(x1),
            edges.get(k + 1).copied().unwrap_or(x1),
        );
        (at(b).saturating_sub(at(a)), b - a)
    };
    let mut made = Vec::new();
    for r in top..=bottom {
        let member = if r == top {
            Some(cell)
        } else {
            cell_at(work, rows.get(r), at(x0))
        };
        let Some(member) = member else { continue };
        let merge = if bottom > top { vmerge(work, member) } else { None };
        let properties = properties_of(work, member);
        let (span, width) = piece(0);
        set_span_width(work, properties, span, Some(width), orders);
        let mut anchor = member;
        for k in 1..n {
            let (span, width) = piece(k);
            let new = new_cell_like(work, member, span, Some(width), merge.as_deref(), orders);
            work.add_next(anchor, new);
            anchor = new;
            made.push(new);
        }
    }
    Ok((made, uneven))
}

/// The new rows below a cell's merge, each with a plain cell under it and every other cell
/// of the row merged down into it.
fn rows_below(
    work: &mut Work,
    last: Id,
    (start, span): (usize, usize),
    count: usize,
    orders: &Orders,
) -> (Vec<Id>, Vec<Id>) {
    let (mut made_rows, mut made_cells) = (Vec::new(), Vec::new());
    let mut anchor = last;
    for _ in 0..count {
        let new_row = make(work, "tr", &[]);
        if let Some(exceptions) = work.find(last, W, "tblPrEx") {
            let copy = work.deep_copy(exceptions);
            work.append(new_row, copy);
        }
        if let Some(found) = work.find(last, W, "trPr") {
            let properties = work.deep_copy(found);
            for child in work.children(properties).to_vec() {
                if work.is_any(child, W, &["ins", "del", "trPrChange", "tblHeader"]) {
                    work.remove(child);
                }
            }
            if !work.children(properties).is_empty() {
                work.append(new_row, properties);
            }
        }
        for placed in row_cells(work, last) {
            let width = find_path(work, placed.cell, &["tcPr", "tcW"]).map(|w| int(work, Some(w), "w", 0));
            let new = if (placed.start, placed.span) == (start, span) {
                new_cell_like(work, placed.cell, placed.span, width, None, orders)
            } else {
                if vmerge(work, placed.cell).is_none() {
                    set_merge(work, placed.cell, Some("restart"), orders);
                }
                new_cell_like(work, placed.cell, placed.span, width, Some("continue"), orders)
            };
            made_cells.push(new);
            work.append(new_row, new);
        }
        work.add_next(anchor, new_row);
        anchor = new_row;
        made_rows.push(new_row);
    }
    (made_rows, made_cells)
}

/// `head` (a cell in its merge's first row) split into `m` down, as Word does
/// (`_split_down`): its vertical merge divided into `m` (the first ones a row more), or,
/// where it has fewer than `m` rows, new rows below its last. The new rows and cells.
fn split_down(work: &mut Work, head: Id, m: usize, orders: &Orders) -> Result<(Vec<Id>, Vec<Id>)> {
    let table = work.root();
    let rows = own_rows(work, table);
    let layout = extents(work, &rows);
    let own = ancestor(work, head, "tr").and_then(|r| rows.iter().position(|x| *x == r));
    let (top, bottom) = layout
        .get(&head)
        .copied()
        .or_else(|| own.map(|r| (r, r)))
        .ok_or_else(off_grid)?;
    let placed = rows
        .get(top)
        .and_then(|r| row_cells(work, *r).into_iter().find(|p| p.cell == head))
        .ok_or_else(off_grid)?;
    let g = bottom - top + 1;
    if g < m {
        let last = *rows.get(bottom).ok_or_else(off_grid)?;
        let made = rows_below(work, last, (placed.start, placed.span), m - g, orders);
        let rows = own_rows(work, table);
        let column: Vec<Id> = (top..top + m)
            .filter_map(|r| cell_at(work, rows.get(r), placed.start))
            .collect();
        for cell in column {
            set_merge(work, cell, None, orders);
        }
        return Ok(made);
    }
    let mut r = top;
    for k in 0..m {
        let size = g / m + usize::from(k < g % m);
        for j in 0..size {
            let Some(cell) = cell_at(work, rows.get(r + j), placed.start) else {
                continue;
            };
            let merge = match (size, j) {
                (1, _) => None,
                (_, 0) => Some("restart"),
                _ => Some("continue"),
            };
            set_merge(work, cell, merge, orders);
        }
        r += size;
    }
    Ok((Vec::new(), Vec::new()))
}

/// `split_cell`: the cell over `(row, column)` split into `rows` x `columns`.
pub(super) fn split_cell(
    sink: &mut EditSink<'_, '_>,
    held: &mut Held,
    split: Split,
    orders: &Orders,
) -> Result<TableChanged> {
    let Split {
        row,
        column,
        rows,
        columns,
    } = split;
    let work = &mut held.work;
    let table = work.root();
    let tracker = matches!(sink.policy(), RevisionPolicy::Tracked(_)).then(|| TableTracker::new(work, table, true));
    let before = own_rows(work, table);
    let found = before
        .get(row)
        .and_then(|r| {
            row_cells(work, *r)
                .into_iter()
                .find(|p| p.start <= column && column < p.start + p.span)
        })
        .ok_or_else(|| EditError::Path(held.found.part.clone()))?;
    let (top, bottom) = extents(work, &before).get(&found.cell).copied().unwrap_or((row, row));
    // a cell merged down is split from its merge's first row
    let cell = cell_at(work, before.get(top), found.start).unwrap_or(found.cell);
    let mut new_cells = Vec::new();
    let mut warnings = Vec::new();
    if columns > 1 {
        let (made, uneven) = split_across(work, &before, (top, bottom), cell, columns, orders)?;
        new_cells.extend(made);
        if let Some((span, sizes)) = uneven {
            let each: Vec<String> = sizes.iter().map(ToString::to_string).collect();
            warnings.push(format!(
                "cell ({row}, {column}) over {span} grid columns split into {columns} along the table's grid, \
                 over {} column(s) each: Word's Split Cells divides the width evenly, adding grid columns and \
                 changing every other row; the grid kept",
                each.join(", ")
            ));
        }
    }
    let top_row = before.get(top).copied();
    let mut heads = vec![cell];
    heads.extend(
        new_cells
            .iter()
            .copied()
            .filter(|c| ancestor(work, *c, "tr") == top_row),
    );
    let mut new_rows = Vec::new();
    if rows > 1 {
        let ordered: Vec<Id> = top_row
            .map(|r| row_cells(work, r))
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.cell)
            .filter(|c| heads.contains(c))
            .collect();
        for head in ordered {
            let (made_rows, made_cells) = split_down(work, head, rows, orders)?;
            new_rows.extend(made_rows);
            new_cells.extend(made_cells);
        }
    }
    let new_paragraphs = paragraphs_made(work, &new_cells, &new_rows);
    let nodes: Vec<Id> = new_rows.iter().chain(new_paragraphs.iter()).copied().collect();
    let raws = stamp_new(sink, held, &nodes, "\0table")?;
    let written = under_policy(sink, |stamp| {
        if let (Some(stamp), Some(tracker)) = (stamp, tracker.as_ref()) {
            let made = (new_rows.as_slice(), new_paragraphs.as_slice());
            track_split(&mut held.work, made, tracker, columns == 1, stamp, orders);
        }
        Ok(())
    })?;
    put(sink, held, written)?;
    sink.ensure_w14(&held.found.part, false, &[])?;
    let wanted: Vec<String> = raws.into_iter().skip(new_rows.len()).collect();
    let created = ids_of(sink, held, &wanted)?;
    Ok(TableChanged {
        id: Some(held.id.clone()),
        created,
        warnings,
        ..TableChanged::default()
    })
}

/// A split recorded as Word tracks it: the new rows and paragraph marks inserted, the other
/// cells' old properties (and, unless a column was divided, the old grid) recorded, a
/// divided merge as `w:cellMerge` where no row was added.
fn track_split(
    work: &mut Work,
    (new_rows, new_paragraphs): (&[Id], &[Id]),
    tracker: &TableTracker,
    grid: bool,
    stamp: &mut crate::policy::Stamp,
    orders: &Orders,
) {
    for new_row in new_rows {
        let properties = match work.find(*new_row, W, "trPr") {
            Some(p) => p,
            None => {
                let p = make(work, "trPr", &[]);
                insert_in_order(work, *new_row, p, orders);
                p
            }
        };
        let record = stamp.make(work, "ins");
        insert_in_order(work, properties, record, orders);
    }
    for paragraph in new_paragraphs {
        if mark_record(work, *paragraph).is_none() {
            mark(work, *paragraph, "ins", stamp, orders);
        }
    }
    // Word's Reject All gives a split column back only without the old grid recorded, and
    // a divided merge only with a w:cellMerge beside the cell's old properties (measured)
    tracker.record(work, stamp, false, grid, orders);
    if new_rows.is_empty() {
        merges_as_cell_merges(work, tracker, stamp, orders);
    }
}

/// The ids of the table's paragraphs whose paraIds are `raws`, in that order.
fn ids_of(sink: &EditSink<'_, '_>, held: &Held, raws: &[String]) -> Result<Vec<String>> {
    let mut found: Vec<(usize, String)> = sink
        .docx()?
        .part(&held.found.part)
        .map(|part| {
            part.ids
                .paragraphs_within(&held.found.path)
                .into_iter()
                .filter_map(|p| Some((raws.iter().position(|w| Some(w) == p.raw.as_ref())?, p.id)))
                .collect()
        })
        .unwrap_or_default();
    found.sort_by_key(|f| f.0);
    Ok(found.into_iter().map(|f| f.1).collect())
}

/// The new cells' paragraphs, then any other paragraph of the new rows.
fn paragraphs_made(work: &Work, new_cells: &[Id], new_rows: &[Id]) -> Vec<Id> {
    let mut out: Vec<Id> = new_cells.iter().flat_map(|c| find_all(work, *c, "p")).collect();
    for new_row in new_rows {
        for paragraph in work.iter(*new_row, W, &["p"]) {
            if !out.contains(&paragraph) {
                out.push(paragraph);
            }
        }
    }
    out
}
