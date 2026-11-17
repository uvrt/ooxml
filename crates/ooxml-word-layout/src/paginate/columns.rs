//! Owns: a page of a document with a section of several text columns (docx2svg's
//! `paginate._lay_columns`, `_fill`, `_balance`, `region_foot`, `_ended_after`): the
//! page's regions one under the other where continuous sections share it, each region's
//! columns filled in turn, a region whose section ends on the page balanced (the least
//! height at which its columns hold it all, by bisection), and where the stack goes on
//! under a region.
//!
//! Must not: stack a column (`scan` does) or decide keeps (`keeps` does). Unequal columns
//! (a paragraph broken again for a narrower column) and notes in columns are lane L3c's.

use super::scan::Frame;
use super::{LaidPage, PageContext, Position, Stopped, Why};
use crate::flow::FlowItem;
use crate::px::Px;
use crate::sections::StoryKind;
use ooxml_base::Q;

/// A region's columns as filled, with the lines and rows they placed.
struct Region {
    frames: Vec<Frame>,
    page: LaidPage,
}

/// Where a region is filled from.
#[derive(Clone)]
struct RegionStart<'s> {
    start: &'s Position,
    story: StoryKind,
    top: Px,
    previous: Option<usize>,
    first_on_page: bool,
    section: usize,
}

fn scratch(start: &Position) -> LaidPage {
    LaidPage {
        page: 0,
        number_before: 0,
        start: start.clone(),
        end: start.clone(),
        reach: start.clone(),
        forced: false,
        lines: Vec::new(),
        rows: Vec::new(),
    }
}

impl PageContext<'_> {
    /// One page of a document with a section of several columns, as its regions.
    #[allow(clippy::result_large_err)]
    pub(super) fn lay_columns_page(&self, start: &Position, story: StoryKind) -> Result<LaidPage, (LaidPage, Stopped)> {
        let mut page = scratch(start);
        let first_section = self.items.get(start.item).map_or(0, FlowItem::section);
        let (top, bottom) = self.edges(first_section, story);
        let mut position = start.clone();
        let mut y = top;
        let mut previous = None;
        let mut first_on_page = true;
        loop {
            let section = self.items.get(position.item).map_or(first_section, FlowItem::section);
            let from = RegionStart {
                start: &position,
                story,
                top: y,
                previous,
                first_on_page,
                section,
            };
            let region = match self.fill(&from, bottom) {
                Ok(region) => region,
                Err((region, stopped)) => {
                    page.lines.extend(region.page.lines);
                    page.rows.extend(region.page.rows);
                    return Err((page, stopped));
                }
            };
            let last_why = region.frames.last().map_or(Why::End, |f| f.why);
            let last_multi = region.frames.last().is_some_and(|f| f.multi);
            let region = if last_multi && last_why == Why::Region {
                self.balance(&from, bottom, region).map_err(|(r, stopped)| {
                    let mut p = page.clone();
                    p.lines.extend(r.page.lines);
                    p.rows.extend(r.page.rows);
                    (p, stopped)
                })?
            } else {
                region
            };
            page.lines.extend(region.page.lines);
            page.rows.extend(region.page.rows);
            page.reach = page.reach.max(region.page.reach);
            let Some(last) = region.frames.last() else {
                return Ok(page);
            };
            if last.why != Why::Region {
                page.end = last.end.clone();
                page.reach = page.reach.clone().max(last.end.clone());
                page.forced = matches!(last.why, Why::Page | Why::Column);
                return Ok(page);
            }
            position = last.end.clone();
            first_on_page = first_on_page && region.frames.iter().all(|f| f.end == f.start);
            (y, previous) = if last.multi {
                self.region_foot(&region.frames)
            } else {
                (last.y, last.last)
            };
        }
    }

    /// One region from its start: its section's columns filled in turn, each from the
    /// region's top down to `bottom`, until the region, the page or the flow ends.
    #[allow(clippy::result_large_err)]
    fn fill(&self, from: &RegionStart<'_>, bottom: Px) -> Result<Region, (Region, Stopped)> {
        let Some(geometry) = self.sections.get(from.section) else {
            return Ok(Region {
                frames: Vec::new(),
                page: scratch(from.start),
            });
        };
        let multi = geometry.multi_column();
        let mut region = Region {
            frames: Vec::new(),
            page: scratch(from.start),
        };
        let mut position = from.start.clone();
        let mut region_top = from.top;
        for (column, b) in geometry.column_boxes().into_iter().enumerate() {
            let top = if column == 0 { from.top } else { region_top };
            let mut frame = Frame::new(from.section, column, top, bottom, position.clone());
            frame.first_on_page = if column == 0 { from.first_on_page } else { true };
            frame.previous = if column == 0 { from.previous } else { None };
            frame.multi = multi;
            frame.offset = b.offset;
            frame.width = b.width;
            frame.region_top = region_top;
            let scanned = self.scan(&position, from.story, Some(&mut frame), &mut region.page);
            match scanned {
                Ok(end) => {
                    region.page.reach = region.page.reach.clone().max(end.reach);
                    position = end.end;
                }
                Err(stopped) => {
                    frame.end = Position::at(stopped.item, stopped.line);
                    region.frames.push(frame);
                    return Err((region, stopped));
                }
            }
            if column == 0 && !from.first_on_page {
                if let Some(first_top) = frame.first_top {
                    region_top = first_top;
                    frame.region_top = region_top;
                }
            }
            let why = frame.why;
            region.frames.push(frame);
            if !matches!(why, Why::Full | Why::Column) || position.item >= self.items.len() {
                break;
            }
        }
        Ok(region)
    }

    /// The region laid out again with its columns balanced: the least height at which its
    /// columns, each filled in turn under the keeps, hold it all (`_balance`).
    #[allow(clippy::result_large_err)]
    fn balance(&self, from: &RegionStart<'_>, bottom: Px, laid: Region) -> Result<Region, (Region, Stopped)> {
        let region_top = laid.frames.first().map_or(from.top, |f| f.region_top);
        let mut lo = Px::ZERO;
        let mut hi = bottom - region_top;
        let step = Px::from(Q::new(1, 65536));
        while hi - lo > step {
            let mid = (lo + hi) / Q::from_integer(2);
            match self.fill(from, region_top + mid) {
                Err(_) => lo = mid,
                Ok(region) => {
                    let holds = region.frames.last().is_some_and(|f| f.why == Why::Region)
                        && region
                            .frames
                            .iter()
                            .filter(|f| f.why == Why::Full)
                            .all(|f| f.end > f.start);
                    if holds {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
            }
        }
        let region = self.fill(from, region_top + hi)?;
        if region.frames.last().is_some_and(|f| f.why != Why::Region) {
            return self.fill(from, bottom);
        }
        Ok(region)
    }

    /// The space after of the last paragraph that ends in `frame` -- a paragraph holding
    /// only a section break left out -- and that item (`_ended_after`).
    fn ended_after(&self, frame: &Frame) -> (Px, Option<usize>) {
        let end = &frame.end;
        if end.line != 0 || !end.part.is_empty() {
            return (Px::ZERO, None);
        }
        let mut index = end.item;
        while index > frame.start.item {
            index -= 1;
            match self.items.get(index) {
                Some(FlowItem::Para(p)) if p.section_mark && index > frame.start.item => continue,
                Some(FlowItem::Para(p)) => return (Px::twips(p.after), Some(index)),
                _ => return (Px::ZERO, Some(index)),
            }
        }
        (Px::ZERO, None)
    }

    /// Where the stack goes on under a region of several columns, and the item the next
    /// paragraph's space before collapses with (`region_foot`).
    fn region_foot(&self, frames: &[Frame]) -> (Px, Option<usize>) {
        let top = frames.first().map_or(Px::ZERO, |f| f.region_top);
        let stacks: Vec<(Px, Px)> = frames
            .iter()
            .map(|f| {
                let stack = if f.end > f.start { f.y - top } else { Px::ZERO };
                (stack, self.ended_after(f).0)
            })
            .collect();
        let height = if self.mode15 {
            stacks.iter().map(|(s, a)| *s + *a).max().unwrap_or(Px::ZERO)
        } else {
            let tallest = stacks.iter().map(|(s, _)| *s).max().unwrap_or(Px::ZERO);
            let count = i128::try_from(frames.len().max(1)).unwrap_or(1);
            let mean = stacks.iter().map(|(s, a)| *s + *a).sum::<Px>() / Q::from_integer(count);
            tallest.max(mean)
        };
        let last = frames.iter().rev().find(|f| f.end > f.start).or(frames.first());
        let (mut after, mut previous) = last.map_or((Px::ZERO, None), |f| self.ended_after(f));
        if previous.is_none() {
            previous = frames.last().and_then(|f| f.end.item.checked_sub(1));
            after = Px::ZERO;
        }
        (top + height - after, previous)
    }
}
