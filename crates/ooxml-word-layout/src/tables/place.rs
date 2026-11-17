//! Owns: what of a table goes on one page ([`place_table`], docx2svg's
//! `table.place_table`): rows whole while they fit with the edge below them; a row split
//! at the page's foot, each cell keeping the lines that fit (two lines each side of a
//! split paragraph in mode 15); a `w:cantSplit` row, a header row and a row that keeps
//! with the next moved whole; header rows repeated on a page the table continues on;
//! merges down going across pages (their lines carried from the page's top) -- each as
//! `make_table_pages_probe.py`, `make_table_foot_probe.py` and
//! `make_table_keep_probe.py` measured it.
//!
//! Must not: measure, or decide where the page ends around the table (the paginator's
//! keeps do).

use super::cells::{row_height, CellFlow, RowFlow};
use super::measured::MeasuredTable;
use crate::px::Px;
use crate::table::{Carried, PlacedRow, TablePage, TableRequest};

/// A cell merged down over more than one row: its first row and cell, its last row and
/// its grid column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeDown {
    /// The merge's first row.
    pub row: usize,
    /// Its cell in that row.
    pub cell: usize,
    /// The last row it reaches.
    pub last: usize,
    /// Its grid column.
    pub column: i64,
}

impl MergeDown {
    /// The merge's first cell, holding its lines.
    fn flow<'t>(&self, table: &'t MeasuredTable) -> Option<&'t CellFlow> {
        table.cell(self.row, self.cell)
    }

    /// The merge's cell in row `r`: its first, or the `continue` one at its column.
    fn index_in(&self, table: &MeasuredTable, r: usize) -> Option<usize> {
        if r == self.row {
            return Some(self.cell);
        }
        table
            .rows
            .get(r)?
            .cells
            .iter()
            .position(|c| c.merged() && c.column == self.column)
    }
}

/// A merge on one page: the first of its lines here, that line's top, the index of its
/// first piece here, and the line after its last here.
#[derive(Debug, Clone, Copy)]
struct OnPage {
    start: usize,
    top: Px,
    first: usize,
    end: Option<usize>,
}

fn u32s(values: &[usize]) -> Vec<u32> {
    values.iter().map(|v| u32::try_from(*v).unwrap_or(u32::MAX)).collect()
}

fn usizes(values: &[u32]) -> Vec<usize> {
    values
        .iter()
        .map(|v| usize::try_from(*v).unwrap_or(usize::MAX))
        .collect()
}

/// One page's placing of a table.
struct Placer<'t> {
    table: &'t MeasuredTable,
    request: &'t TableRequest,
    pieces: Vec<PlacedRow>,
    state: Vec<Option<OnPage>>,
    y: Px,
}

/// What of a row goes on the page: its part from `start` to `end`, `height` tall.
struct Piece {
    row: usize,
    start: Vec<usize>,
    end: Vec<usize>,
    height: Px,
    continued: bool,
    split: bool,
    overflowed: bool,
}

impl<'t> Placer<'t> {
    fn push(&mut self, piece: Piece, header: bool) {
        let top_border = self.table.rows.get(piece.row).map_or(Px::ZERO, |r| r.top_border);
        self.pieces.push(PlacedRow {
            row: piece.row,
            header,
            top: self.y,
            top_border,
            height: piece.height,
            start: u32s(&piece.start),
            end: u32s(&piece.end),
            continued: piece.continued,
            split: piece.split,
            carry: Vec::new(),
            overflowed: piece.overflowed,
        });
        self.y += top_border + piece.height;
    }

    fn merge(&self, k: usize) -> Option<(&'t MergeDown, &'t CellFlow, OnPage)> {
        let m = self.table.merges.get(k)?;
        let found = self.state.get(k).copied().flatten()?;
        Some((m, m.flow(self.table)?, found))
    }

    fn set_end(&mut self, k: usize, end: usize) {
        if let Some(Some(found)) = self.state.get_mut(k) {
            found.end = Some(end);
        }
    }

    /// The page as placed: each merge on it given the lines that end above the last piece
    /// where its end is not known yet, and its carry where it goes across the page.
    fn finish(mut self, end: Px, next: Option<(usize, Vec<u32>)>) -> TablePage {
        for k in 0..self.table.merges.len() {
            let Some((_, merged, found)) = self.merge(k) else {
                continue;
            };
            if found.end.is_some() {
                continue;
            }
            let limit = self.pieces.last().map_or(self.request.y, PlacedRow::bottom) - merged.margin_bottom;
            let fitting = merged.fitting(found.start, found.top, limit);
            self.set_end(k, fitting);
        }
        let rows = self.carried();
        TablePage { end, next, rows }
    }

    /// Nothing more of the table on this page: it goes on at `next`.
    fn leave(self, next: (usize, Vec<u32>)) -> TablePage {
        let headers = self.table.header_rows;
        if self.request.mode15
            && headers > 0
            && !self.pieces.is_empty()
            && !self.request.first_on_page
            && self.pieces.iter().all(|p| p.row < headers && !p.header)
        {
            // Mode 15: header rows are not left at the foot with no row after them.
            return TablePage {
                end: self.request.y,
                next: Some((0, Vec::new())),
                rows: Vec::new(),
            };
        }
        let edge = self.pieces.last().map_or(Px::ZERO, |p| self.table.edge_below(p.row));
        let end = self.y + edge;
        self.finish(end, Some(next))
    }

    /// The pieces with each merge that does not lie whole on this page given its lines
    /// here, on the first of its pieces here (`_carried`).
    fn carried(&self) -> Vec<PlacedRow> {
        let mut out = self.pieces.clone();
        for k in 0..self.table.merges.len() {
            let Some((m, merged, found)) = self.merge(k) else {
                continue;
            };
            let Some(piece) = out.get_mut(found.first) else {
                continue;
            };
            let end = found.end.unwrap_or(found.start);
            let (start32, end32) = (
                u32::try_from(found.start).unwrap_or(u32::MAX),
                u32::try_from(end).unwrap_or(u32::MAX),
            );
            let whole = piece.row == m.row && !piece.continued && found.start == 0 && end == merged.stack.len();
            if whole {
                if let Some(slot) = piece.end.get_mut(m.cell) {
                    *slot = end32;
                }
                continue;
            }
            if piece.row == m.row {
                if let Some(slot) = piece.start.get_mut(m.cell) {
                    *slot = start32;
                }
                if let Some(slot) = piece.end.get_mut(m.cell) {
                    *slot = start32;
                }
            }
            piece.carry.push(Carried {
                row: m.row,
                cell: m.cell,
                start: start32,
                end: end32,
            });
        }
        out
    }

    /// How many of a merge's lines its rows above `row` hold, laid out one after another
    /// (`_placed_before`).
    fn placed_before(&self, m: &MergeDown, merged: &CellFlow, row: usize) -> usize {
        let rows = &self.table.rows;
        let heights: Px = rows.get(m.row..row).unwrap_or_default().iter().map(row_height).sum();
        let borders: Px = rows
            .get(m.row + 1..row)
            .unwrap_or_default()
            .iter()
            .map(|r| r.top_border)
            .sum();
        merged.fitting(0, merged.margin_top, heights + borders - merged.margin_bottom)
    }

    /// The merges active in row `row`, each started on this page where it was not yet.
    fn activate(&mut self, row: usize, starts: &[usize], continued: bool, content_top: Px) -> Vec<usize> {
        let mut active = Vec::new();
        for (k, m) in self.table.merges.iter().enumerate() {
            if !(m.row <= row && row <= m.last) {
                continue;
            }
            active.push(k);
            if self.state.get(k).copied().flatten().is_some() {
                continue;
            }
            let Some(merged) = m.flow(self.table) else { continue };
            let index = m.index_in(self.table, row);
            let start = match index {
                Some(i) if continued => starts.get(i).copied().unwrap_or(0),
                _ if row == m.row => 0,
                _ => self.placed_before(m, merged, row),
            };
            let on_page = OnPage {
                start,
                top: content_top + merged.margin_top,
                first: self.pieces.len(),
                end: None,
            };
            if let Some(slot) = self.state.get_mut(k) {
                *slot = Some(on_page);
            }
        }
        active
    }
}

/// Per cell, how many of its lines from `starts` fit in `room`, its margins included;
/// with `widow_control` (mode 15) a paragraph split there keeps two lines on each side,
/// or moves whole (`_split_counts`).
fn split_counts(row: &RowFlow, starts: &[usize], room: Px, widow_control: bool) -> Vec<usize> {
    row.cells
        .iter()
        .zip(starts)
        .map(|(cell, &start)| {
            if cell.merging() {
                return 0;
            }
            let mut count = 0;
            while start + count < cell.stack.len()
                && cell.margins() + cell.slice_height(start, start + count + 1) <= room
            {
                count += 1;
            }
            let mut count = i64::try_from(count).unwrap_or(0);
            let at = start + usize::try_from(count).unwrap_or(0);
            if widow_control && count > 0 && at < cell.stack.len() {
                let line = cell.stack.get(at);
                let entry = line.and_then(|l| cell.entries.get(l.entry));
                if let (Some(line), Some(entry)) = (line, entry) {
                    if entry.widow.unwrap_or(true) {
                        let lines = i64::try_from(entry.pitches.len()).unwrap_or(0);
                        let number = i64::try_from(line.number).unwrap_or(0);
                        let (start, at) = (i64::try_from(start).unwrap_or(0), i64::try_from(at).unwrap_or(0));
                        let first = at - number;
                        let mut on_page = at - first.max(start);
                        let left = lines - number;
                        if left < 2 && lines >= 2 {
                            count -= 2 - left;
                            on_page -= 2 - left;
                        }
                        if 0 < on_page && on_page < 2 && first >= start {
                            count -= on_page;
                        }
                    }
                }
            }
            usize::try_from(count.max(0)).unwrap_or(0)
        })
        .collect()
}

/// One row as the page meets it.
struct RowAt<'t> {
    row: usize,
    current: &'t RowFlow,
    starts: Vec<usize>,
    ends: Vec<usize>,
    continued: bool,
    content_top: Px,
    active: Vec<usize>,
    height: Px,
    edge: Px,
}

impl RowAt<'_> {
    fn whole(&self) -> Piece {
        Piece {
            row: self.row,
            start: self.starts.clone(),
            end: self.ends.clone(),
            height: self.height,
            continued: self.continued,
            split: false,
            overflowed: false,
        }
    }
}

/// What placing one row came to.
enum Step {
    /// The table goes on at the next row.
    Next,
    /// The page's part of the table is placed.
    Done(TablePage),
}

impl<'t> Placer<'t> {
    /// The header rows repeated at the top of a page the table continues on.
    fn repeat_headers(&mut self) {
        let (table, request) = (self.table, self.request);
        let headers = table.header_rows;
        let continues = request.row > 0 || !request.part.is_empty();
        if !(request.first_on_page && continues && 0 < headers && headers <= request.row) {
            return;
        }
        for (h, repeated) in table.rows.iter().enumerate().take(headers) {
            let ends: Vec<usize> = repeated.cells.iter().map(|c| c.stack.len()).collect();
            let piece = Piece {
                row: h,
                start: vec![0; ends.len()],
                end: ends,
                height: repeated.height,
                continued: false,
                split: false,
                overflowed: false,
            };
            self.push(piece, true);
        }
    }

    /// Row `row` as the page meets it, from each cell's line `part` where it began on an
    /// earlier page: its merges started here and its height.
    fn meet(&mut self, row: usize, current: &'t RowFlow, part: Vec<usize>) -> RowAt<'t> {
        let ends: Vec<usize> = current.cells.iter().map(|c| c.stack.len()).collect();
        let continued = !part.is_empty();
        let starts = if continued { part } else { vec![0; ends.len()] };
        let content_top = self.y + current.top_border;
        let active = self.activate(row, &starts, continued, content_top);
        let mut height = if continued {
            current
                .cells
                .iter()
                .zip(&starts)
                .filter(|(c, _)| !c.merging())
                .map(|(c, s)| c.margins() + c.slice_height(*s, c.stack.len()))
                .max()
                .unwrap_or(Px::ZERO)
        } else if !active.is_empty() {
            row_height(current)
        } else {
            current.height
        };
        for &k in &active {
            if let Some((m, merged, found)) = self.merge(k) {
                if m.last == row {
                    let reach = found.top + merged.slice_height(found.start, merged.stack.len()) + merged.margin_bottom;
                    height = height.max(reach - content_top);
                }
            }
        }
        RowAt {
            row,
            current,
            starts,
            ends,
            continued,
            content_top,
            active,
            height,
            edge: self.table.edge_below(row),
        }
    }

    /// Place row `at` on the page: whole where it fits, else moved, split or (on an empty
    /// page) placed past the foot.
    fn place_row(&mut self, at: &RowAt<'_>) -> Option<TablePage> {
        let (table, request) = (self.table, self.request);
        if self.y + at.current.top_border + at.height + at.edge <= request.bottom {
            self.push(at.whole(), false);
            self.end_merges(&at.active, at.row);
            return None;
        }
        let nothing_yet = request.first_on_page && !self.pieces.iter().any(|piece| !piece.header);
        let part = if at.continued { u32s(&at.starts) } else { Vec::new() };
        let header = at.row < table.header_rows && !at.continued;
        let cant_split = at.current.cant_split;
        if !(cant_split || header || nothing_yet || request.split_kept) && table.row_keeps(at.row) {
            return Some(self.leave_here((at.row, part)));
        }
        if (cant_split || header) && !(nothing_yet && request.mode15) {
            if nothing_yet {
                self.push(at.whole(), false);
                return None;
            }
            return Some(self.leave_here((at.row, part)));
        }
        Some(self.split(at, nothing_yet, part))
    }

    /// Row `at` split at the page's foot: each cell keeping the lines that fit, the
    /// merges' lines that end above the break kept; or the row moved, or placed past the
    /// foot of an empty page.
    fn split(&mut self, at: &RowAt<'_>, nothing_yet: bool, part: Vec<u32>) -> TablePage {
        let (table, request) = (self.table, self.request);
        let room = request.bottom - at.edge - self.y - at.current.top_border;
        let mut counts = split_counts(at.current, &at.starts, room, request.mode15);
        let holds_back = at
            .current
            .cells
            .iter()
            .zip(counts.iter().zip(&at.starts))
            .any(|(cell, (count, start))| !cell.merging() && *count == 0 && *start < cell.stack.len());
        if holds_back {
            counts = vec![0; counts.len()];
        }
        let split_ends: Vec<usize> = at.starts.iter().zip(&counts).map(|(s, c)| s + c).collect();
        let mut split_height = at
            .current
            .cells
            .iter()
            .zip(at.starts.iter().zip(&split_ends))
            .filter(|(c, _)| !c.merging())
            .map(|(c, (s, e))| c.margins() + c.slice_height(*s, *e))
            .max()
            .unwrap_or(Px::ZERO);
        let kept = self.merges_kept(at, &mut split_height);
        let grows = at.active.iter().any(|&k| {
            self.merge(k).is_some_and(|(m, merged, found)| {
                at.row == m.last
                    && kept.get(k).copied().unwrap_or(0)
                        > merged.fitting(found.start, found.top, self.y - merged.margin_bottom)
            })
        });
        if counts.iter().all(|c| *c == 0) && !grows {
            if !nothing_yet {
                return self.leave_here((at.row, part));
            }
            // Not one line of it fits on an empty page: placed whole, past the foot.
            let mut piece = at.whole();
            piece.overflowed = true;
            self.push(piece, false);
            self.end_merges(&at.active, at.row);
            let next = (at.row + 1 < table.rows.len()).then(|| (at.row + 1, Vec::new()));
            return self.finish_here(self.y + at.edge, next);
        }
        let piece = Piece {
            row: at.row,
            start: at.starts.clone(),
            end: split_ends.clone(),
            height: split_height,
            continued: at.continued,
            split: true,
            overflowed: false,
        };
        self.push(piece, false);
        for &k in &at.active {
            self.set_end(k, kept.get(k).copied().unwrap_or(0));
        }
        let mut next = split_ends;
        for &k in &at.active {
            let Some(m) = table.merges.get(k) else { continue };
            let end = self.state.get(k).copied().flatten().and_then(|f| f.end);
            if let (Some(index), Some(end)) = (m.index_in(table, at.row), end) {
                if let Some(slot) = next.get_mut(index) {
                    *slot = end;
                }
            }
        }
        self.finish_here(self.y + at.edge, Some((at.row, u32s(&next))))
    }

    /// Per merge active in the row, its lines that end above the page's break: within the
    /// row's part for a merge going on below it, down to the page's foot for one the row
    /// ends (which may make the part taller).
    fn merges_kept(&self, at: &RowAt<'_>, split_height: &mut Px) -> Vec<usize> {
        let request = self.request;
        let mut kept = vec![0; self.table.merges.len()];
        for &k in &at.active {
            let Some((m, merged, found)) = self.merge(k) else {
                continue;
            };
            let margin = merged.margin_bottom;
            let limit = if at.row == m.last {
                request.bottom - at.edge
            } else {
                at.content_top + *split_height
            } - margin;
            let fit = merged.fitting(found.start, found.top, limit);
            if let Some(slot) = kept.get_mut(k) {
                *slot = fit;
            }
            if at.row == m.last && fit > found.start {
                let reach = found.top + merged.slice_height(found.start, fit) + margin - at.content_top;
                *split_height = (*split_height).max(reach);
            }
        }
        kept
    }

    /// Each active merge that ends at `row` placed to its last line.
    fn end_merges(&mut self, active: &[usize], row: usize) {
        for &k in active {
            let Some(m) = self.table.merges.get(k) else { continue };
            if m.last == row {
                let len = m.flow(self.table).map_or(0, |c| c.stack.len());
                self.set_end(k, len);
            }
        }
    }

    fn leave_here(&mut self, next: (usize, Vec<u32>)) -> TablePage {
        let placer = self.take();
        placer.leave(next)
    }

    fn finish_here(&mut self, end: Px, next: Option<(usize, Vec<u32>)>) -> TablePage {
        let placer = self.take();
        placer.finish(end, next)
    }

    /// This placer's state, leaving an empty one behind.
    fn take(&mut self) -> Placer<'t> {
        Placer {
            table: self.table,
            request: self.request,
            pieces: std::mem::take(&mut self.pieces),
            state: std::mem::take(&mut self.state),
            y: self.y,
        }
    }

    fn step(&mut self, row: usize, part: Vec<usize>) -> Step {
        let Some(current) = self.table.rows.get(row) else {
            let end = self.y + self.table.bottom_border;
            return Step::Done(self.finish_here(end, None));
        };
        let at = self.meet(row, current, part);
        match self.place_row(&at) {
            Some(page) => Step::Done(page),
            None => Step::Next,
        }
    }
}

/// What of a table goes on the page from `request.y`, starting at `request.row` (from
/// each cell's stacked line `request.part` where the row began on an earlier page), the
/// page's text ending at `request.bottom` (`place_table`).
pub fn place_table(table: &MeasuredTable, request: &TableRequest) -> TablePage {
    let mut p = Placer {
        table,
        request,
        pieces: Vec::new(),
        state: vec![None; table.merges.len()],
        y: request.y,
    };
    p.repeat_headers();
    let mut row = request.row;
    let mut part = usizes(&request.part);
    loop {
        match p.step(row, std::mem::take(&mut part)) {
            Step::Next => row += 1,
            Step::Done(page) => return page,
        }
    }
}
