//! Owns: one page (or one text column) stacked from a start (docx2svg's `paginate._scan`):
//! paragraphs' lines placed under their spacing and borders until one does not fit, a
//! page or column break, a section that starts a page or a continuous one of other
//! columns; tables placed through their [`crate::table::TableFlow`]; the footnotes at the
//! foot (a line referencing notes fits with them under it, `notes`); where the stack
//! ends and why.
//!
//! Must not: decide where a page ends past an overflow (`keeps` does), or lay out more
//! than one column (`columns` does).

use super::floats::{clear_of_bands, Band, Floats, Top, Tops};
use super::notes::{Fitted, Foot};
use super::{
    LaidPage, NoteRest, PageContext, PageStory, PlacedLine, PlacedNote, PlacedTableRow, Position, Stopped, Why,
};
use crate::flow::{FlowItem, ParaItem, TableItem};
use crate::notes::NoteLines;
use crate::px::Px;
use crate::rebreak::Lines;
use crate::table::TableRequest;
use crate::vertical::{keeps_space_before_at_page_top, page_top_gap, paragraph_gap};

/// One text column of a page as laid out (`paginate.Frame`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    pub section: usize,
    pub column: usize,
    pub top: Px,
    pub bottom: Px,
    /// Empty above its first line.
    pub first_on_page: bool,
    /// The item before its first, where it goes on from a region above.
    pub previous: Option<usize>,
    /// Its section has several columns.
    pub multi: bool,
    /// Its offset from the text area's left edge and its width, twips.
    pub offset: i64,
    pub width: i64,
    pub start: Position,
    pub end: Position,
    /// Where its stack ends.
    pub y: Px,
    pub why: Why,
    pub region_top: Px,
    /// The top of its first paragraph, past the space between it and the one before.
    pub first_top: Option<Px>,
    /// The item the stack's next paragraph spaces itself from.
    pub last: Option<usize>,
    /// It starts at the page's top.
    pub page_top: bool,
    /// In mode 15, a column of several whose notes go to the page's note area.
    pub pooled: bool,
    /// The footnotes at its foot (below mode 15), or referenced in it (pooled).
    pub notes: Vec<PlacedNote>,
    /// The lines of notes the column (or page) before could not hold, at its foot first.
    pub carried: NoteRest,
}

impl Frame {
    /// A column of `section` from `start`, `top` to `bottom`.
    pub fn new(section: usize, column: usize, top: Px, bottom: Px, start: Position) -> Frame {
        Frame {
            section,
            column,
            top,
            bottom,
            first_on_page: true,
            previous: None,
            multi: false,
            offset: 0,
            width: 0,
            end: start.clone(),
            start,
            y: top,
            why: Why::Full,
            region_top: top,
            first_top: None,
            last: None,
            page_top: false,
            pooled: false,
            notes: Vec::new(),
            carried: std::sync::Arc::from(Vec::new()),
        }
    }
}

/// How one scan ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScanEnd {
    pub end: Position,
    pub forced: bool,
    pub why: Why,
    /// The furthest position the scan read.
    pub reach: Position,
}

/// Where a scan is: the item and line it is at, the scan's start, the page's (or
/// column's) bottom and the column.
struct At<'s> {
    index: usize,
    line: usize,
    start: &'s Position,
    bottom: Px,
    frame: Option<&'s Frame>,
}

/// One scan's moving state.
pub(super) struct Stack<'o> {
    y: Px,
    /// The top of the first paragraph or table, past the space before it.
    first_top: Option<Px>,
    previous: Option<usize>,
    first_on_page: bool,
    stack_ends: Vec<(Position, Px)>,
    out: &'o mut LaidPage,
    /// Where this scan's lines and rows start in `out`.
    lines_from: usize,
    rows_from: usize,
    /// The page's (or column's) foot: its notes as placed.
    pub(super) foot: Foot,
    /// The bands its lines keep off.
    bands: Vec<Band>,
    /// The drawings its lines go beside.
    wraps: &'o [crate::wrap::Wrap],
    /// The lines it placed, for the drawings anchored on them.
    tops: &'o mut Tops,
}

/// A step of the stack: go on with the next item, or end the scan.
enum Step {
    Next,
    Done(ScanEnd),
}

/// A line placed in `frame` (none: a page of one column).
fn placed(frame: Option<&Frame>, item: usize, line: usize, top: Px, pitch: Px, paragraph_top: Px) -> PlacedLine {
    PlacedLine {
        item,
        line,
        top,
        pitch,
        paragraph_top,
        column: frame.filter(|f| f.multi).map(|f| f.column),
        offset: frame.filter(|f| f.multi).map_or(0, |f| f.offset),
    }
}

fn done(end: Position, forced: bool, why: Why, reach: Position) -> Step {
    Step::Done(ScanEnd {
        end,
        forced,
        why,
        reach,
    })
}

impl PageContext<'_> {
    /// One page of a document of one-column sections from `start`.
    #[allow(clippy::result_large_err)]
    pub(super) fn lay_one_column_page(
        &self,
        start: &Position,
        story: PageStory,
        carry: &NoteRest,
        page_index: usize,
    ) -> Result<LaidPage, (LaidPage, Stopped)> {
        let laid = |floats: Floats<'_>, tops: &mut Tops| {
            let mut page = LaidPage::empty(start, std::sync::Arc::clone(carry));
            let scanned = self.scan(start, story, None, &mut page, floats, tops);
            match scanned {
                Ok(end) => {
                    page.end = end.end;
                    page.reach = end.reach;
                    page.forced = end.forced;
                    Ok(page)
                }
                Err(stopped) => Err((page, stopped)),
            }
        };
        // Laid out once without the bands of its drawings, which are positioned from that
        // layout, and again with them.
        let mut tops = Tops::default();
        let first = laid(Floats::default(), &mut tops);
        let mut bands = self.page_bands(&tops, page_index);
        let mut stacked_reach = start.clone();
        if bands.len() > 1 && first.is_ok() {
            let (stacked, reach) = self.stacked_bands(start, story, carry, page_index, bands);
            bands = stacked;
            stacked_reach = reach;
        }
        let wraps = if self.wraps {
            self.page_wraps(&tops, page_index)
        } else {
            Vec::new()
        };
        if bands.is_empty() && wraps.is_empty() {
            return first;
        }
        let first_reach = match &first {
            Ok(page) | Err((page, _)) => page.reach.clone(),
        };
        let again = laid(
            Floats {
                bands: &bands,
                wraps: &wraps,
            },
            &mut Tops::default(),
        );
        let mut page = match (first, again) {
            (Err((_, stopped)), Ok(page) | Err((page, _))) => return Err((with_floats(page, bands, wraps), stopped)),
            (Ok(_), Err((page, stopped))) => return Err((with_floats(page, bands, wraps), stopped)),
            (Ok(_), Ok(page)) => page,
        };
        page.bands = bands;
        page.wraps = wraps;
        // The page reaches as far as its layout without its drawings read.
        page.reach = page.reach.clone().max(first_reach).max(stacked_reach);
        Ok(page)
    }

    /// The page's bands, each drawing positioned from the page laid out with the bands of
    /// the drawings before it (`_stacked_bands`): one whose anchor the bands before it
    /// push off the page goes with its paragraph, and so do those after it; and how far
    /// those layouts read.
    fn stacked_bands(
        &self,
        start: &Position,
        story: PageStory,
        carry: &NoteRest,
        page_index: usize,
        bands: Vec<Band>,
    ) -> (Vec<Band>, Position) {
        let mut reach = start.clone();
        let mut fixed: Vec<Band> = bands.iter().take(1).copied().collect();
        for band in bands.iter().skip(1) {
            let mut found = Tops::default();
            let mut scratch = LaidPage::empty(start, std::sync::Arc::clone(carry));
            let floats = Floats {
                bands: &fixed,
                wraps: &[],
            };
            match self.scan(start, story, None, &mut scratch, floats, &mut found) {
                Ok(end) => reach = reach.max(end.reach),
                Err(_) => break,
            }
            let again = self.page_bands(&found, page_index);
            match again.into_iter().find(|b| b.key == band.key) {
                Some(placed) => fixed.push(placed),
                None => break,
            }
        }
        (fixed, reach)
    }

    /// Stack one page or column from `start` (`_scan`); its lines, rows and footnotes go
    /// to `out` (a column's footnotes to its frame), the notes the page before could not
    /// hold being `out.carried` (a column's, its frame's).
    pub(super) fn scan(
        &self,
        start: &Position,
        story: PageStory,
        frame: Option<&mut Frame>,
        out: &mut LaidPage,
        floats: Floats<'_>,
        tops: &mut Tops,
    ) -> Result<ScanEnd, Stopped> {
        let first_section = self.items.get(start.item).map_or(0, FlowItem::section);
        let (top, bottom) = match frame.as_deref() {
            Some(f) => (f.top, f.bottom),
            None => self.edges(first_section, story),
        };
        let carry = frame
            .as_deref()
            .map_or_else(|| std::sync::Arc::clone(&out.carried), |f| f.carried.clone());
        let foot = Foot::new(self, first_section, frame.as_deref(), carry);
        let mut top = top;
        let opens_endnotes = match self.items.get(start.item) {
            Some(FlowItem::Para(p)) => p.continuation.filter(|_| frame.as_deref().is_none_or(|f| f.page_top)),
            _ => None,
        };
        if let Some(pitch) = opens_endnotes {
            // A page the endnotes go on to: the continuation separator's line first.
            if frame.is_none() {
                out.opens = Some(super::OpeningLine { top, pitch });
            }
            top += pitch;
        }
        let mut stack = Stack {
            y: top,
            first_top: None,
            previous: frame.as_deref().and_then(|f| f.previous),
            first_on_page: frame.as_deref().is_none_or(|f| f.first_on_page),
            stack_ends: Vec::new(),
            lines_from: out.lines.len(),
            rows_from: out.rows.len(),
            out,
            foot,
            bands: across(floats.bands, frame.as_deref(), self),
            wraps: floats.wraps,
            tops,
        };
        let (mut index, mut line) = (start.item, start.line);
        let end = loop {
            let Some(item) = self.items.get(index) else {
                let end = Position::at(self.items.len(), 0);
                break ScanEnd {
                    reach: end.clone(),
                    end,
                    forced: false,
                    why: Why::End,
                };
            };
            if let (Some(f), 0) = (frame.as_deref(), line) {
                if let Some(end) = self.section_ends_column(f, item, index) {
                    break end;
                }
            }
            let at = At {
                index,
                line,
                start,
                bottom,
                frame: frame.as_deref(),
            };
            let step = match item {
                FlowItem::Stop(s) => {
                    return Err(Stopped {
                        item: index,
                        line: 0,
                        reason: s.reason.clone(),
                    })
                }
                FlowItem::Table(t) => self.scan_table(t, &at, &mut stack)?,
                FlowItem::Para(p) => self.scan_para(p, &at, &mut stack),
            };
            match step {
                Step::Next => {
                    index += 1;
                    line = 0;
                }
                Step::Done(end) => break end,
            }
        };
        finish(stack, &end, frame);
        Ok(end)
    }

    /// In a column, an item of another section: a section that starts a page ends the
    /// column and the page; a continuous one of other columns, the region.
    fn section_ends_column(&self, frame: &Frame, item: &FlowItem, index: usize) -> Option<ScanEnd> {
        if item.section() == frame.section {
            return None;
        }
        let at = Position::at(index, 0);
        if self.sections.new_page(item.section()) {
            return Some(ScanEnd {
                end: at.clone(),
                forced: true,
                why: Why::Page,
                reach: at,
            });
        }
        let multi = self.sections.get(item.section()).is_some_and(|s| s.multi_column());
        (frame.multi || multi).then(|| ScanEnd {
            end: at.clone(),
            forced: false,
            why: Why::Region,
            reach: at,
        })
    }

    /// The gap above a paragraph's first line, and where its box starts.
    fn first_line_gap(&self, p: &ParaItem, index: usize, stack: &Stack<'_>) -> (Px, Px) {
        if stack.first_on_page {
            let keeps = keeps_space_before_at_page_top(p.section_start, p.page_break_before, self.mode15);
            let above = index.checked_sub(1).and_then(|k| self.items.get(k));
            let previous_after = above.and_then(FlowItem::para).map_or(0, |a| a.after);
            return (page_top_gap(p.before, keeps, previous_after).0, stack.y);
        }
        match stack.previous.and_then(|k| self.items.get(k)) {
            Some(FlowItem::Para(previous)) => {
                let same = p.style == previous.style;
                let own_before = if p.contextual && same { 0 } else { p.before };
                let prev_after = if previous.contextual && same { 0 } else { previous.after };
                (paragraph_gap(prev_after, own_before).0, stack.y + Px::twips(prev_after))
            }
            _ => (Px::twips(p.before), stack.y),
        }
    }

    /// Where a paragraph that does not start the page ends it first: a section that
    /// starts a page, `pageBreakBefore`; or, holding only a section break, where it is
    /// placed taking no room.
    fn para_entry(&self, p: &ParaItem, at: &At<'_>, stack: &mut Stack<'_>) -> Option<Step> {
        let (index, frame) = (at.index, at.frame);
        if !stack.first_on_page && at.line == 0 {
            let previous_section = stack.previous.and_then(|k| self.items.get(k)).map(FlowItem::section);
            let new_section = previous_section.is_some_and(|s| s != p.section) && self.sections.new_page(p.section);
            if new_section || p.page_break_before {
                let at = Position::at(index, 0);
                return Some(done(at.clone(), true, Why::Page, at));
            }
        }
        let previous_is_table = matches!(stack.previous.and_then(|k| self.items.get(k)), Some(FlowItem::Table(_)));
        if p.section_mark && !stack.first_on_page && !(self.mode15 && previous_is_table) {
            // On this page, and taking no room: the stack goes on as if it were not there.
            // It is drawn where it stands, past its space before (the page walk's).
            let pitch = self.lines_of(index, p).heights().first().map_or(Px::ZERO, |h| h.pitch);
            let (gap, box_top) = self.first_line_gap(p, index, stack);
            let line = placed(frame, index, 0, stack.y + gap + p.top_border, pitch, box_top);
            stack.out.lines.push(line);
            stack.foot.mark(Position::at(index, 0));
            let y = stack.y;
            stack.tops.record(top_of(frame, index, 0, (y, pitch, y)));
            return Some(Step::Next);
        }
        None
    }

    fn scan_para(&self, p: &ParaItem, at: &At<'_>, stack: &mut Stack<'_>) -> Step {
        if let Some(step) = self.para_entry(p, at, stack) {
            return step;
        }
        let mut paragraph_top = stack.y;
        if let Some(source) = &p.source {
            // Lines an earlier layout of this page broke beside a drawing are broken again
            // in the whole column.
            let current = self.lines_of(at.index, p);
            self.broken(at.index, self.breaker.plain(source, &current, at.line));
        }
        let mut number = at.line;
        while number < self.lines_of(at.index, p).count() {
            if number == 0 {
                let (gap, box_top) = self.first_line_gap(p, at.index, stack);
                paragraph_top = box_top;
                stack.y += gap + p.top_border;
                if stack.first_top.is_none() {
                    stack.first_top = Some(stack.y - p.top_border);
                }
            }
            if let Some(step) = self.place_line(p, at, stack, number, paragraph_top) {
                return step;
            }
            number += 1;
        }
        stack.y += p.border;
        stack.previous = Some(at.index);
        Step::Next
    }

    /// Line `number` of a paragraph placed at the stack's foot, its footnotes at the
    /// page's; where it does not fit, or ends the page or column, how the scan ends.
    fn place_line(
        &self,
        p: &ParaItem,
        at: &At<'_>,
        stack: &mut Stack<'_>,
        number: usize,
        paragraph_top: Px,
    ) -> Option<Step> {
        let (index, start, frame) = (at.index, at.start, at.frame);
        if let (Some(source), Some(f)) = (&p.source, frame.filter(|f| f.multi)) {
            // One of unequal columns: the line is as wide as its column.
            let current = self.lines_of(index, p);
            self.broken(index, self.breaker.fit_width(source, &current, number, f.width));
        }
        let lines = self.lines_of(index, p);
        let height = *lines.heights().get(number)?;
        let height = &height;
        let count = lines.count();
        let last = number + 1 == count;
        if !stack.bands.is_empty() {
            // Below mode 15 a paragraph's last line reaches down to the end of its space
            // after; in mode 15 to the end of its pitch.
            let after = if last && !self.mode15 {
                Px::twips(p.after)
            } else {
                Px::ZERO
            };
            stack.y = clear_of_bands(stack.y, height.pitch + after, &stack.bands);
        }
        if let (false, Some(source)) = (stack.wraps.is_empty(), &p.source) {
            stack.y = self.beside(p, at, stack, source, number);
        }
        // The line as broken beside the drawings.
        let lines = self.lines_of(index, p);
        let height = *lines.heights().get(number)?;
        let height = &height;
        let count = lines.count();
        let last = number + 1 == count;
        let border = if last || !self.mode15 { p.border } else { Px::ZERO };
        let after = if last { Px::twips(p.after) } else { Px::ZERO };
        let fitted = Fitted {
            y: stack.y,
            height,
            border,
            after,
            bottom: at.bottom,
        };
        let mut new_notes = lines.notes_on(number);
        if let Some(f) = frame.filter(|f| f.multi && !f.pooled) {
            // In one of unequal columns, a note broken in the column's width.
            for note in &mut new_notes {
                let id = note.first().and_then(|l| l.note);
                if let Some(found) = id.and_then(|id| self.notes_at.get(&(id, p.section, f.width))) {
                    *note = NoteLines::clone(found);
                }
            }
        }
        let need = stack.foot.need(new_notes.iter(), &fitted);
        if !stack.first_on_page && !p.section_mark && stack.y + height.fit() + border > at.bottom - need {
            let overflow = self.hyphen_overflow(p, at, &lines, number);
            let empty_ok = frame.is_some_and(|f| !f.first_on_page);
            let end = self.break_before(start, overflow.clone(), empty_ok);
            return Some(done(end, false, Why::Full, overflow));
        }
        stack.foot.place(new_notes.iter(), &fitted);
        stack.foot.mark(Position::at(index, number));
        stack
            .tops
            .record(top_of(frame, index, number, (stack.y, height.pitch, paragraph_top)));
        stack
            .out
            .lines
            .push(placed(frame, index, number, stack.y, height.pitch, paragraph_top));
        stack.y += height.pitch;
        let end_y = stack.y + if last { p.border } else { Px::ZERO };
        stack.stack_ends.push((Position::at(index, number), end_y));
        stack.first_on_page = false;
        let next = if number + 1 < count {
            Position::at(index, number + 1)
        } else {
            Position::at(index + 1, 0)
        };
        if lines.measured().page_breaks.contains(&number) {
            return Some(done(next, true, Why::Page, Position::at(index, number)));
        }
        if lines.measured().column_breaks.contains(&number) {
            return Some(done(next, true, Why::Column, Position::at(index, number)));
        }
        None
    }

    /// Line `number` broken beside the drawings it reaches into, and where its top is.
    fn beside(
        &self,
        p: &ParaItem,
        at: &At<'_>,
        stack: &Stack<'_>,
        source: &crate::rebreak::ParaSource,
        number: usize,
    ) -> Px {
        let current = self.lines_of(at.index, p);
        let column = at.frame.filter(|f| f.multi).map(|f| (f.offset, f.width));
        let Some(section) = self.sections.get(p.section) else {
            return stack.y;
        };
        let beside = crate::rebreak::Beside {
            number,
            last: number + 1 == current.count(),
            after: Px::twips(p.after).value(),
            wraps: stack.wraps,
            column: super::floats::wrap_column(section, self.mode15, column),
        };
        let (y, found) = self.breaker.fit_beside(source, &current, &beside, stack.y.value());
        self.broken(at.index, found);
        Px::from(y)
    }

    /// Where a page ends whose line `number` does not fit, past an automatic hyphen ending
    /// the line before: in mode 15 that line goes too -- or, where Word moves only the
    /// word, the line keeps its text before the word and the next starts with it.
    fn hyphen_overflow(&self, p: &ParaItem, at: &At<'_>, lines: &Lines<'_>, number: usize) -> Position {
        let (index, start) = (at.index, at.start);
        let at_hyphen = at.frame.is_none()
            && number > 0
            && lines.measured().auto_hyphen_ends.contains(&(number - 1))
            && Position::at(index, number - 1) > *start;
        if !at_hyphen {
            return Position::at(index, number);
        }
        let moved = match (&p.source, self.hyphen_at_page_end) {
            (Some(source), super::HyphenMove::Word) if source.section.is_none() => {
                let found = self.breaker.move_hyphenated_word(source, lines, number - 1);
                let moved = found.is_some();
                self.broken(index, found);
                moved
            }
            _ => false,
        };
        match self.hyphen_at_page_end {
            super::HyphenMove::Nothing => Position::at(index, number),
            _ if moved => Position::at(index, number),
            _ => Position::at(index, number - 1),
        }
    }

    /// The space between the item before and a table (`table_gap_px`).
    fn table_gap(&self, previous: Option<usize>, t: &TableItem) -> Px {
        table_gap(previous.and_then(|k| self.items.get(k)), t)
    }
}

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
    fn scan_table(&self, t: &TableItem, at: &At<'_>, stack: &mut Stack<'_>) -> Result<Step, Stopped> {
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
        let mut request = TableRequest {
            y: stack.y,
            bottom,
            row: line,
            part,
            first_on_page: stack.first_on_page,
            mode15: self.mode15,
            split_kept: false,
        };
        let stopped = |why: crate::table::Unsupported| Stopped {
            item: index,
            line,
            reason: why.0,
        };
        let mut laid = t.flow.place(&request).map_err(stopped)?;
        if self.kept_row_breaks(t, &laid, index, start, frame) {
            request.split_kept = true;
            laid = t.flow.place(&request).map_err(stopped)?;
        }
        record_rows(stack, index, &laid);
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
fn across(bands: &[Band], frame: Option<&Frame>, cx: &PageContext<'_>) -> Vec<Band> {
    let Some(f) = frame.filter(|f| f.multi && !bands.is_empty()) else {
        return bands.to_vec();
    };
    let left = cx.sections.get(f.section).map_or(Px::ZERO, |s| {
        let m = &s.props.margins;
        Px::from(ooxml_base::Q::new(i128::from(m.left + m.gutter + f.offset) * 300, 1440))
    });
    let right = left + Px::from(ooxml_base::Q::new(i128::from(f.width) * 300, 1440));
    bands
        .iter()
        .filter(|b| b.span.is_none_or(|(l, r)| l < right && r > left))
        .copied()
        .collect()
}

/// A line placed at `(top, pitch, paragraph top)`, as a drawing anchored on it is
/// positioned against it.
fn top_of(frame: Option<&Frame>, item: usize, line: usize, at: (Px, Px, Px)) -> Top {
    Top {
        item,
        line,
        top: at.0,
        pitch: at.1,
        paragraph_top: at.2,
        column: frame.filter(|f| f.multi).map(|f| (f.offset, f.width)),
    }
}

/// `page` with the bands and wraps its lines kept off.
fn with_floats(mut page: LaidPage, bands: Vec<Band>, wraps: Vec<crate::wrap::Wrap>) -> LaidPage {
    page.bands = bands;
    page.wraps = wraps;
    page
}

/// A scan's end: what the page holds ends where the next starts (lines and rows the keeps
/// sent on are dropped), and the column records where its stack ends.
fn finish(stack: Stack<'_>, end: &ScanEnd, frame: Option<&mut Frame>) {
    // What the page holds ends where the next starts.
    let lines_from = stack.lines_from;
    let kept: Vec<PlacedLine> = stack
        .out
        .lines
        .drain(lines_from..)
        .filter(|l| Position::at(l.item, l.line) < end.end)
        .collect();
    stack.out.lines.extend(kept);
    let rows_from = stack.rows_from;
    let rows: Vec<PlacedTableRow> = stack
        .out
        .rows
        .drain(rows_from..)
        .filter(|r| {
            let at = Position {
                item: r.item,
                line: r.row.row,
                part: r.row.part(),
            };
            r.row.header || at < end.end
        })
        .collect();
    stack.out.rows.extend(rows);
    let notes = stack.foot.kept(&end.end);
    if let Some(f) = frame {
        f.notes = notes;
        if f.first_top.is_none() {
            f.first_top = stack.first_top;
        }
        f.why = end.why;
        f.end = end.end.clone();
        f.last = stack.previous;
        f.y = stack
            .stack_ends
            .iter()
            .rev()
            .find(|(position, _)| *position < end.end)
            .map_or(f.top, |(_, y)| *y);
    } else {
        // What of its last footnote the page cannot hold goes on to the next (`_carried`).
        stack.out.carry_out = notes
            .last()
            .map_or_else(|| std::sync::Arc::from(Vec::new()), |n| n.rest().into());
        stack.out.notes = notes;
    }
}

/// A table's rows as placed on the page: each recorded, and where the stack ends after it.
fn record_rows(stack: &mut Stack<'_>, index: usize, laid: &crate::table::TablePage) {
    for row in &laid.rows {
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
            row: row.clone(),
        });
    }
}
