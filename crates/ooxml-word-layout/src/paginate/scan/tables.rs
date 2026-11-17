//! Owns: a table stacked on a page or column (docx2svg's `paginate._scan`, its tables'
//! part): the space above it, its rows placed through its [`crate::table::TableFlow`] --
//! split, kept with the next, its header rows repeated -- beside drawings in a room of
//! their own, and where the page ends inside it.
//!
//! Must not: stack paragraphs (the scan does) or decide keeps between paragraphs.

use super::{done, At, Frame, Stack, Step};
use crate::flow::{FlowItem, TableItem};
use crate::paginate::{PageContext, PlacedTableRow, Position, Stopped, Why};
use crate::px::Px;
use crate::table::TableRequest;

/// The space between `previous` and a table after it (`table_gap_px`): the paragraph's
/// space after, whole, unless contextual against the table's first paragraph's style;
/// between two tables, one edge.
pub(crate) fn table_gap(previous: Option<&FlowItem>, t: &TableItem) -> Px {
    match previous {
        Some(FlowItem::Table(before)) => {
            if t.flow.rows() == 0 {
                Px::ZERO
            } else {
                -before.flow.bottom_border().min(t.flow.top_border())
            }
        }
        Some(FlowItem::Para(before)) => {
            if before.contextual && t.flow.opens_with_style(before.style.as_deref()) {
                Px::ZERO
            } else {
                Px::twips(before.after)
            }
        }
        _ => Px::ZERO,
    }
}

impl PageContext<'_> {
    pub(super) fn scan_table(&self, t: &TableItem, at: &At<'_>, stack: &mut Stack<'_>) -> Result<Step, Stopped> {
        let At {
            index,
            line,
            start,
            bottom,
            frame,
        } = *at;
        let previous_section = stack.previous.and_then(|k| self.items.get(k)).map(FlowItem::section);
        if !stack.first_on_page
            && line == 0
            && start.part.is_empty()
            && previous_section.is_some_and(|s| s != t.section)
            && self.sections.new_page(t.section)
        {
            let at = Position::at(index, 0);
            return Ok(done(at.clone(), true, Why::Page, at));
        }
        if line == 0 && !stack.first_on_page {
            stack.y += self.table_gap(stack.previous, t);
        }
        if stack.first_top.is_none() {
            stack.first_top = Some(stack.y);
        }
        let part = if index == start.item {
            start.part.clone()
        } else {
            Vec::new()
        };
        let request = TableRequest {
            y: stack.y,
            // Above the notes already at the page's foot.
            bottom: bottom - stack.foot.taken(),
            row: line,
            part,
            first_on_page: stack.first_on_page,
            mode15: self.mode15,
            split_kept: false,
            stop: None,
        };
        let placed = self.place_table_page(t, at, stack, request)?;
        record_rows(stack, index, &placed, frame.filter(|f| f.multi).map(|f| f.column));
        let laid = placed.page;
        for row in laid.rows.iter().filter(|r| !r.header) {
            stack.foot.mark(Position {
                item: index,
                line: row.row,
                part: row.part(),
            });
        }
        if let Some((next_row, next_part)) = laid.next.clone() {
            return Ok(self.table_page_end(&laid, at, next_row, next_part));
        }
        stack.y = laid.end;
        stack.first_on_page = false;
        stack.previous = Some(index);
        Ok(Step::Next)
    }

    /// A table's part of the page from `request`: beside the drawings on the page or in
    /// its column where it goes there, its cells' footnotes at the page's foot, a row that
    /// keeps with the next split where keeping it would leave the page empty.
    fn place_table_page(
        &self,
        t: &TableItem,
        at: &At<'_>,
        stack: &mut Stack<'_>,
        mut request: TableRequest,
    ) -> Result<crate::paginate::beside::Placed, Stopped> {
        let At {
            index,
            line,
            start,
            bottom,
            frame,
        } = *at;
        let stopped = |why: crate::table::Unsupported| Stopped {
            item: index,
            line,
            reason: why.0,
        };
        let beside = match frame.filter(|f| f.offset != 0) {
            // In a column right of the first: the table laid out in that column.
            Some(f) => Some(crate::paginate::beside::Beside {
                row: 0,
                room: self.room(index, f.offset, f.width),
                drop: None,
            }),
            // Beside the drawings text wraps around on this page.
            None if line == 0 && request.part.is_empty() && !stack.wraps.is_empty() => self
                .table_beside(t, index, stack.y, stack.wraps, stack.first_on_page)
                .map_err(stopped)?,
            None => None,
        };
        let noted = frame.is_none_or(|f| !f.multi);
        let mut placed;
        loop {
            placed = self.place_beside(t, beside.as_ref(), &request).map_err(stopped)?;
            let mut foot = stack.foot.clone();
            if noted {
                // Footnotes referenced in its cells: each line holding one fits as a body
                // line does, its notes under it at the page's foot.
                (placed, foot) = self
                    .table_notes(t, placed, &request, beside.as_ref(), &stack.foot, bottom)
                    .map_err(stopped)?;
            }
            if request.split_kept || !self.kept_row_breaks(t, &placed.page, index, start, frame) {
                stack.foot = foot;
                break;
            }
            request.split_kept = true;
        }
        Ok(placed)
    }

    /// Where a page ends inside a table: before a row the keeps send on, or inside a row
    /// split across the page's foot.
    fn table_page_end(
        &self,
        laid: &crate::table::TablePage,
        at: &At<'_>,
        next_row: usize,
        next_part: Vec<u32>,
    ) -> Step {
        let reach = Position {
            item: at.index,
            line: next_row,
            part: next_part.clone(),
        };
        if !laid.any_body() {
            let end = self.break_before(at.start, Position::at(at.index, next_row), false);
            return done(end, false, Why::Full, reach);
        }
        if !next_part.is_empty() {
            return done(reach.clone(), false, Why::Full, reach);
        }
        let empty_ok = at.frame.is_some_and(|f| !f.first_on_page);
        let end = self.break_before(at.start, Position::at(at.index, next_row), empty_ok);
        done(end, false, Why::Full, reach)
    }

    /// Whether a row that keeps with the next, moved whole to the next page, is split
    /// after all: below mode 15, where its chain of keeps reaches back to the page's start
    /// (`_kept_row_breaks`).
    fn kept_row_breaks(
        &self,
        t: &TableItem,
        laid: &crate::table::TablePage,
        index: usize,
        start: &Position,
        frame: Option<&Frame>,
    ) -> bool {
        if self.mode15 {
            return false;
        }
        let Some((row, part)) = &laid.next else { return false };
        if !part.is_empty() || *row >= t.flow.rows() || frame.is_some_and(|f| !f.first_on_page) {
            return false;
        }
        t.flow.keeps_with_next(*row) && self.keeps(start, Position::at(index, *row)) <= *start
    }
}

/// The bands of `bands` a column's lines keep off: in a column of several, those of
/// drawings across it.
/// A table's rows as placed on the page: each recorded, and where the stack ends after it.
pub(super) fn record_rows(
    stack: &mut Stack<'_>,
    index: usize,
    placed: &crate::paginate::beside::Placed,
    column: Option<usize>,
) {
    for (k, row) in placed.page.rows.iter().enumerate() {
        if !row.header {
            let position = Position {
                item: index,
                line: row.row,
                part: row.part(),
            };
            stack.stack_ends.push((position, row.bottom()));
        }
        stack.out.rows.push(PlacedTableRow {
            item: index,
            column,
            row: row.clone(),
            room: placed.room.clone().filter(|_| k >= placed.from),
        });
    }
}
