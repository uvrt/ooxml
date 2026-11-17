//! Owns: a page of a document with a section of several text columns (docx2svg's
//! `paginate._lay_columns`, `_fill`, `_balance`, `region_foot`, `_ended_after`): the
//! page's regions one under the other where continuous sections share it, each region's
//! columns filled in turn, a region whose section ends on the page balanced (the least
//! height at which its columns hold it all, by bisection), and where the stack goes on
//! under a region.
//!
//! Below mode 15 each column's footnotes stand at its own foot, what one cannot hold going
//! on at the next one's (and from the last, at the next page's).
//!
//! Must not: stack a column (`scan` does) or decide keeps (`keeps` does).

use super::scan::Frame;
use super::{note_rest, LaidPage, NoteRest, PageContext, Position, Stopped, Why};
use crate::flow::FlowItem;
use crate::px::Px;
use crate::sections::StoryKind;
use ooxml_base::Q;

/// A region's columns as filled, with the lines and rows they placed.
#[derive(Clone)]
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
    /// Its first column is the page's first (`not done`, at the page's top).
    page_top: bool,
    /// The lines of notes the page before could not hold (the page's first region only).
    carry: NoteRest,
}

/// A page's notes: a page whose notes stand at its foot lists them (a page of several
/// columns, none: they are its columns'); what its last column (or its last note) could
/// not hold goes on to the next page.
fn page_notes(page: &mut LaidPage, frames: &[Frame]) {
    if frames.iter().any(|f| f.multi) {
        page.notes.clear();
    } else {
        page.notes = frames.iter().flat_map(|f| f.notes.iter().cloned()).collect();
    }
    let last_placed = frames.iter().rev().find(|f| f.end > f.start);
    page.carry_out = match frames.last() {
        Some(last) if last.multi && !last.pooled => note_rest(&last.notes),
        _ => last_placed
            .and_then(|f| f.notes.last())
            .map_or_else(|| std::sync::Arc::from(Vec::new()), |n| n.rest().into()),
    };
}

fn scratch(start: &Position) -> LaidPage {
    LaidPage::empty(start, std::sync::Arc::from(Vec::new()))
}

impl PageContext<'_> {
    /// One page of a document with a section of several columns, as its regions; in mode
    /// 15 its columns end above the room its note area takes, laid out again until that
    /// room holds (`_lay_columns_page`, `_lay_noted`): should it not settle, the larger.
    #[allow(clippy::result_large_err)]
    pub(super) fn lay_columns_page(
        &self,
        start: &Position,
        story: StoryKind,
        carry: &NoteRest,
    ) -> Result<LaidPage, (LaidPage, Stopped)> {
        let mut room = Px::ZERO;
        let mut seen: Vec<Px> = Vec::new();
        loop {
            let (page, frames) = self.lay_regions(start, story, carry, room)?;
            let pooled: Vec<&Frame> = frames.iter().filter(|f| f.pooled).collect();
            let Some(first) = pooled.first() else {
                return Ok(page);
            };
            let ids: Vec<i64> = pooled
                .iter()
                .flat_map(|f| f.notes.iter().filter_map(|n| n.lines.first().and_then(|l| l.note)))
                .collect();
            let mut need = if ids.is_empty() {
                Px::ZERO
            } else {
                // A note that cannot be laid out takes no room (docx2svg says so, and goes on).
                self.note_area(first.section, &ids).unwrap_or(Px::ZERO)
            };
            if need >= first.bottom + room - first.top {
                // Notes taller than the page: laid out without their room.
                need = Px::ZERO;
            }
            if need == room {
                return Ok(page);
            }
            seen.push(room);
            if seen.contains(&need) || seen.len() > 8 {
                let larger = seen.iter().copied().fold(need, Px::max);
                return self.lay_regions(start, story, carry, larger).map(|(page, _)| page);
            }
            room = need;
        }
    }

    /// The page's regions, its columns ending `note_room` above its foot; and its frames.
    #[allow(clippy::result_large_err)]
    fn lay_regions(
        &self,
        start: &Position,
        story: StoryKind,
        carry: &NoteRest,
        note_room: Px,
    ) -> Result<(LaidPage, Vec<Frame>), (LaidPage, Stopped)> {
        let mut page = LaidPage::empty(start, std::sync::Arc::clone(carry));
        let first_section = self.items.get(start.item).map_or(0, FlowItem::section);
        let (top, bottom) = self.edges(first_section, story);
        let bottom = bottom - note_room;
        let mut position = start.clone();
        let mut y = top;
        if let Some(FlowItem::Para(p)) = self.items.get(start.item) {
            // A page the endnotes go on to: the continuation separator's line first.
            if let Some(pitch) = p.continuation {
                page.opens = Some(super::OpeningLine { top: y, pitch });
                y += pitch;
            }
        }
        let mut frames: Vec<Frame> = Vec::new();
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
                page_top: frames.is_empty() && first_on_page,
                carry: if frames.is_empty() {
                    std::sync::Arc::clone(carry)
                } else {
                    std::sync::Arc::from(Vec::new())
                },
            };
            let region = self.fill(&from, bottom).and_then(|region| {
                let last = region.frames.last();
                if last.is_some_and(|f| f.multi && f.why == Why::Region) {
                    self.balance(&from, bottom, region)
                } else {
                    Ok(region)
                }
            });
            let region = match region {
                Ok(region) => region,
                Err((region, stopped)) => {
                    page.lines.extend(region.page.lines);
                    page.rows.extend(region.page.rows);
                    return Err((page, stopped));
                }
            };
            page.lines.extend(region.page.lines);
            page.rows.extend(region.page.rows);
            page.reach = page.reach.max(region.page.reach);
            let Some(last) = region.frames.last().cloned() else {
                return Ok((page, frames));
            };
            first_on_page = first_on_page && region.frames.iter().all(|f| f.end == f.start);
            let foot = if last.multi {
                self.region_foot(&region.frames)
            } else {
                (last.y, last.last)
            };
            frames.extend(region.frames);
            if last.why != Why::Region {
                page.end = last.end.clone();
                page.reach = page.reach.clone().max(last.end.clone());
                page.forced = matches!(last.why, Why::Page | Why::Column);
                page_notes(&mut page, &frames);
                return Ok((page, frames));
            }
            position = last.end.clone();
            (y, previous) = foot;
        }
    }

    /// The room a page's note area takes for notes `ids` in section `section`'s columns:
    /// their paragraphs a flow of their own, balanced in the columns (the least height at
    /// which the columns, filled in turn under the keeps, hold them all), and the
    /// separator's line above (`note_area`); `None` where a note cannot be laid out.
    fn note_area(&self, section: usize, ids: &[i64]) -> Option<Px> {
        let mut items: Vec<FlowItem> = Vec::new();
        for id in ids {
            for item in self.note_items.get(&(*id, section))?.iter() {
                let item = match item {
                    // Only the area's first paragraph starts its section.
                    FlowItem::Para(p) if p.section_start && !items.is_empty() => {
                        FlowItem::Para(std::sync::Arc::new(crate::flow::ParaItem {
                            section_start: false,
                            ..(**p).clone()
                        }))
                    }
                    FlowItem::Para(_) => item.clone(),
                    _ => return None,
                };
                items.push(item);
            }
        }
        let props = &self.sections.get(section)?.props;
        let sections = crate::sections::Sections::new(&[crate::notes::flowed::note_section(props, items.len())], false);
        let none = std::collections::BTreeMap::new();
        let no_notes = crate::flow::NoteItems::new();
        let cx = PageContext {
            items: &items,
            sections: &sections,
            mode15: self.mode15,
            hyphen_at_page_end: self.hyphen_at_page_end,
            stories: &crate::stories::NoStories,
            separators: &none,
            note_items: &no_notes,
        };
        let start = Position::default();
        let from = RegionStart {
            start: &start,
            story: StoryKind::Default,
            top: Px::ZERO,
            previous: None,
            first_on_page: true,
            section: 0,
            page_top: true,
            carry: std::sync::Arc::from(Vec::new()),
        };
        let holds = |height: Px| -> Option<Region> {
            let region = cx.fill(&from, height).ok()?;
            region.frames.last().is_some_and(|f| f.end.item >= items.len()).then_some(region)
        };
        let (mut lo, mut hi) = (Px::ZERO, Px::int(100_000));
        holds(hi)?;
        let step = Px::from(Q::new(1, 65536));
        while hi - lo > step {
            let mid = (lo + hi) / Q::from_integer(2);
            if holds(mid).is_some() {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let height = holds(hi)?.frames.iter().map(|f| f.y).max()?;
        Some(height + self.separators(section).0)
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
            frame.page_top = from.page_top && column == 0;
            frame.pooled = multi && self.mode15;
            frame.carried = match region.frames.last() {
                // Below mode 15 the notes the column before could not hold go on at this
                // one's foot.
                Some(before) if multi && !frame.pooled => note_rest(&before.notes),
                Some(_) => std::sync::Arc::from(Vec::new()),
                None => std::sync::Arc::clone(&from.carry),
            };
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
