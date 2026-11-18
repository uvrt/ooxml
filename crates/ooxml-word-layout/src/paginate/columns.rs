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

use super::floats::{Floats, Tops};
use super::scan::Frame;
use super::PageStory;
use super::{note_rest, LaidPage, NoteRest, PageContext, Position, Stopped, Why};
use crate::flow::{FlowItem, ParaItem};
use crate::px::Px;
use crate::sections::StoryKind;
use ooxml_base::Q;

/// Where a page starts: its first line, the story kind it shows, the lines of notes the
/// page before could not hold.
struct PageStart<'s> {
    start: &'s Position,
    story: PageStory,
    carry: &'s NoteRest,
}

/// A page's regions as laid out: the page, its columns, the lines they placed, and where
/// the layout stopped, if it did.
struct Regions {
    page: LaidPage,
    frames: Vec<Frame>,
    tops: Tops,
    stopped: Option<Stopped>,
}

/// A region's columns as filled, with the lines and rows they placed.
#[derive(Clone)]
struct Region {
    frames: Vec<Frame>,
    page: LaidPage,
    /// The lines its columns placed.
    tops: Tops,
    /// Where the layout stopped in it, if it did.
    stopped: Option<Stopped>,
}

/// Where a region is filled from.
#[derive(Clone)]
struct RegionStart<'s> {
    start: &'s Position,
    story: PageStory,
    top: Px,
    previous: Option<usize>,
    first_on_page: bool,
    section: usize,
    /// Its first column is the page's first (`not done`, at the page's top).
    page_top: bool,
    /// The lines of notes the page before could not hold (the page's first region only).
    carry: NoteRest,
    /// What its lines keep off.
    floats: Floats<'s>,
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
    /// Laid out once without its drawings' bands, which are positioned from that layout,
    /// and again with them.
    #[allow(clippy::result_large_err)]
    pub(super) fn lay_columns_page(
        &self,
        start: &Position,
        story: PageStory,
        carry: &NoteRest,
        page_index: usize,
    ) -> Result<LaidPage, (LaidPage, Stopped)> {
        let page = PageStart { start, story, carry };
        let (first, room) = self.lay_noted(&page);
        let bands = self.page_bands(&first.tops, page_index);
        let mut wraps = if self.wraps {
            self.page_wraps(&first.tops, page_index)
        } else {
            Vec::new()
        };
        let laid = if bands.is_empty() && wraps.is_empty() {
            first
        } else {
            let mut again;
            let mut attempt = 0;
            loop {
                again = self.lay_regions(
                    &page,
                    room,
                    Floats {
                        bands: &bands,
                        wraps: &wraps,
                    },
                );
                if again.stopped.is_some() {
                    break;
                }
                // A drop cap in a column is positioned where the text above it ends with
                // the drop caps above it in place: again until they hold.
                let found = if self.wraps {
                    self.page_wraps(&again.tops, page_index)
                } else {
                    Vec::new()
                };
                let in_columns = found.iter().any(|w| {
                    w.drop_cap
                        .is_some_and(|item| again.tops.iter().any(|t| t.item == item && t.column.is_some()))
                });
                let edges = |ws: &[crate::wrap::Wrap]| {
                    ws.iter()
                        .map(|w| (w.top, w.bottom, w.left, w.right))
                        .collect::<Vec<_>>()
                };
                attempt += 1;
                if !in_columns || edges(&found) == edges(&wraps) || attempt == 4 {
                    break;
                }
                wraps = found;
            }
            again.page.bands = bands;
            again.page.wraps = wraps;
            again.page.reach = again.page.reach.clone().max(first.page.reach.clone());
            again.stopped = first.stopped.or(again.stopped);
            again
        };
        let mut laid = laid;
        self.record_columns(&mut laid);
        match laid.stopped {
            Some(stopped) => Err((laid.page, stopped)),
            None => Ok(laid.page),
        }
    }

    /// The page's columns as its drawing reads them, where its body's stack ends (its last
    /// column's), and in mode 15 its note area.
    fn record_columns(&self, laid: &mut Regions) {
        let page = &mut laid.page;
        page.columns = laid
            .frames
            .iter()
            .map(|f| super::PageColumn {
                section: f.section,
                column: f.column,
                multi: f.multi,
                pooled: f.pooled,
                offset: f.offset,
                width: f.width,
                region_top: f.region_top,
                y: f.y,
                filled: f.end > f.start,
                notes: f.notes.clone(),
                carried: std::sync::Arc::clone(&f.carried),
            })
            .collect();
        if let Some(last) = laid.frames.last() {
            page.body_end = last.y;
        }
        let pooled: Vec<&Frame> = laid.frames.iter().filter(|f| f.pooled).collect();
        let ids: Vec<i64> = pooled
            .iter()
            .flat_map(|f| f.notes.iter().filter_map(|n| n.lines.first().and_then(|l| l.note)))
            .collect();
        if let (Some(first), false) = (pooled.first(), ids.is_empty()) {
            page.note_area = self.note_region(first.section, &ids).map(std::sync::Arc::new);
        }
    }

    /// The page's regions with the room of its note area found (`_lay_noted`); the page
    /// reaches as far as any of its layouts read.
    fn lay_noted(&self, page: &PageStart<'_>) -> (Regions, Px) {
        let mut reach = page.start.clone();
        let reached = |mut laid: Regions, reach: &Position| {
            laid.page.reach = laid.page.reach.clone().max(reach.clone());
            laid
        };
        let mut room = Px::ZERO;
        let mut seen: Vec<Px> = Vec::new();
        loop {
            let laid = self.lay_regions(page, room, Floats::default());
            reach = reach.max(laid.page.reach.clone());
            if laid.stopped.is_some() {
                return (reached(laid, &reach), room);
            }
            let pooled: Vec<&Frame> = laid.frames.iter().filter(|f| f.pooled).collect();
            let Some(first) = pooled.first() else {
                return (reached(laid, &reach), room);
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
                return (reached(laid, &reach), room);
            }
            seen.push(room);
            if seen.contains(&need) || seen.len() > 8 {
                let larger = seen.iter().copied().fold(need, Px::max);
                return (
                    reached(self.lay_regions(page, larger, Floats::default()), &reach),
                    larger,
                );
            }
            room = need;
        }
    }

    /// The page's regions, its columns ending `note_room` above its foot, their lines
    /// keeping off `floats`.
    fn lay_regions(&self, at: &PageStart<'_>, note_room: Px, floats: Floats<'_>) -> Regions {
        let (start, story, carry) = (at.start, at.story, at.carry);
        let mut out = Regions {
            page: LaidPage::empty(start, std::sync::Arc::clone(carry)),
            frames: Vec::new(),
            tops: Tops::default(),
            stopped: None,
        };
        let page = &mut out.page;
        let first_section = self.items.get(start.item).map_or(0, FlowItem::section);
        let (top, bottom) = self.edges(first_section, story);
        let bottom = bottom - note_room;
        let mut position = start.clone();
        let mut y = top;
        if let Some(pitch) = self
            .items
            .get(start.item)
            .and_then(FlowItem::para)
            .and_then(ParaItem::continuation_pitch)
        {
            // A page the endnotes go on to: the continuation separator's line first.
            page.opens = Some(super::OpeningLine { top: y, pitch });
            y += pitch;
        }
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
                page_top: out.frames.is_empty() && first_on_page,
                carry: if out.frames.is_empty() {
                    std::sync::Arc::clone(carry)
                } else {
                    std::sync::Arc::from(Vec::new())
                },
                floats,
            };
            let region = self.region(&from, bottom);
            page.lines.extend(region.page.lines);
            page.rows.extend(region.page.rows);
            out.tops.extend(region.tops);
            if region.stopped.is_some() {
                out.frames.extend(region.frames);
                out.stopped = region.stopped;
                return out;
            }
            page.reach = page.reach.clone().max(region.page.reach);
            let Some(last) = region.frames.last().cloned() else {
                return out;
            };
            first_on_page = first_on_page && region.frames.iter().all(|f| f.end == f.start);
            let foot = if last.multi {
                self.region_foot(&region.frames)
            } else {
                (last.y, last.last)
            };
            out.frames.extend(region.frames);
            if last.why != Why::Region {
                page.end = last.end.clone();
                page.reach = page.reach.clone().max(last.end.clone());
                page.forced = matches!(last.why, Why::Page | Why::Column);
                page_notes(page, &out.frames);
                return out;
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
        let area = self.note_region(section, ids)?;
        Some(area.height + self.separators(section).0)
    }

    /// The note area for notes `ids` in section `section`'s columns, laid out
    /// (`note_area`): its flow, its lines from its top, its height without the separator.
    fn note_region(&self, section: usize, ids: &[i64]) -> Option<super::NoteArea> {
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
        let no_pages = super::PageStories::new();
        let no_notes = crate::flow::NoteItems::new();
        let no_widths = crate::flow::NotesAt::new();
        let cx = PageContext {
            items: &items,
            sections: &sections,
            mode15: self.mode15,
            hyphen_at_page_end: self.hyphen_at_page_end,
            stories: &crate::stories::NoStories,
            page_stories: &no_pages,
            separators: &none,
            note_items: &no_notes,
            notes_at: &no_widths,
            wraps: false,
            breaker: self.breaker,
            rooms: None,
            lines: std::cell::RefCell::default(),
        };
        let start = Position::default();
        let from = RegionStart {
            start: &start,
            story: PageStory {
                kind: StoryKind::Default,
                page: 0,
            },
            top: Px::ZERO,
            previous: None,
            first_on_page: true,
            section: 0,
            page_top: true,
            carry: std::sync::Arc::from(Vec::new()),
            floats: Floats::default(),
        };
        let holds = |height: Px| -> Option<Region> {
            let region = cx.fill(&from, height);
            let ended = region.frames.last().is_some_and(|f| f.end.item >= items.len());
            (region.stopped.is_none() && ended).then_some(region)
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
        // Its paragraphs broken again as the last fill broke them.
        cx.lines.borrow_mut().clear();
        let region = holds(hi)?;
        let height = region.frames.iter().map(|f| f.y).max()?;
        Some(super::NoteArea {
            section,
            notes: ids.to_vec(),
            lines: region.page.lines,
            height,
            broken: cx.lines.take(),
        })
    }

    /// One region from its start: its section's columns filled in turn, each from the
    /// region's top down to `bottom`, until the region, the page or the flow ends.
    #[allow(clippy::result_large_err)]
    fn fill(&self, from: &RegionStart<'_>, bottom: Px) -> Region {
        let mut region = Region {
            frames: Vec::new(),
            page: scratch(from.start),
            tops: Tops::default(),
            stopped: None,
        };
        let Some(geometry) = self.sections.get(from.section) else {
            return region;
        };
        let multi = geometry.multi_column();
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
            let scanned = self.scan(
                &position,
                from.story,
                Some(&mut frame),
                &mut region.page,
                from.floats,
                &mut region.tops,
            );
            match scanned {
                Ok(end) => {
                    region.page.reach = region.page.reach.clone().max(end.reach);
                    position = end.end;
                }
                Err(stopped) => {
                    frame.end = Position::at(stopped.item, stopped.line);
                    region.frames.push(frame);
                    region.stopped = Some(stopped);
                    return region;
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
        region
    }

    /// One region, balanced where its section ends on the page and a continuous one
    /// follows.
    fn region(&self, from: &RegionStart<'_>, bottom: Px) -> Region {
        let region = self.fill(from, bottom);
        let last = region.frames.last();
        if region.stopped.is_none() && last.is_some_and(|f| f.multi && f.why == Why::Region) {
            self.balance(from, bottom, region)
        } else {
            region
        }
    }

    /// The region laid out again with its columns balanced: the least height at which its
    /// columns, each filled in turn under the keeps, hold it all (`_balance`).
    #[allow(clippy::result_large_err)]
    fn balance(&self, from: &RegionStart<'_>, bottom: Px, laid: Region) -> Region {
        let region_top = laid.frames.first().map_or(from.top, |f| f.region_top);
        let mut lo = Px::ZERO;
        let mut hi = bottom - region_top;
        let step = Px::from(Q::new(1, 65536));
        while hi - lo > step {
            let mid = (lo + hi) / Q::from_integer(2);
            let region = self.fill(from, region_top + mid);
            let holds = region.stopped.is_none()
                && region.frames.last().is_some_and(|f| f.why == Why::Region)
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
        let mut region = self.fill(from, region_top + hi);
        if region.stopped.is_none() && region.frames.last().is_some_and(|f| f.why != Why::Region) {
            region = self.fill(from, bottom);
        }
        // The lines its first filling placed are positioned against too.
        let mut tops = laid.tops;
        tops.extend(std::mem::take(&mut region.tops));
        region.tops = tops;
        region
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
