//! Owns: pagination (docx2svg's `paginate.paginate`, `_lay_page` and what they share):
//! positions in the flow ([`Position`]), what a page is beyond its lines ([`PageInfo`]),
//! each laid page with its lines' and rows' places ([`LaidPage`], [`PlacedLine`]), and the
//! loop over pages -- page numbers, blank pages before odd and even sections, story kinds
//! -- from the flow's start or from a page's checkpoint until a caller says the rest is
//! known ([`Paginator`]). One page is laid out by `scan` (one column) or `columns` (a
//! document with a section of several columns); where it ends when a line does not fit
//! is `keeps`'.
//!
//! A page's footnotes stand at its foot ([`PlacedNote`]): each line referencing notes
//! fits with them under it, and what of a note a page cannot hold goes on at the next
//! page's foot ([`NoteRest`]) -- on a page of its own in mode 15 after a forced break.
//!
//! Must not: measure (the flow is measured), lay tables out (a [`crate::table::TableFlow`]
//! places a table's rows), or lay out headers and footers (their heights come through
//! [`crate::stories::StoryHeights`]).

mod columns;
mod keeps;
mod notes;
mod scan;

use crate::flow::FlowItem;
use crate::notes::{NoteLine, NoteLines};
use crate::px::Px;
use crate::sections::{Sections, StoryKind};
use crate::stories::StoryHeights;
use crate::table::PlacedRow;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The lines of a note a page (or column) could not hold, which go on at the next one's
/// foot under the continuation separator.
pub type NoteRest = Arc<[NoteLine]>;

/// A footnote at a page's (or column's) foot: its lines and how many of them are there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedNote {
    /// The note's lines.
    pub lines: NoteLines,
    /// How many of them are on the page.
    pub taken: usize,
}

impl PlacedNote {
    /// The lines that go on to the next page or column.
    pub fn rest(&self) -> &[NoteLine] {
        self.lines.get(self.taken..).unwrap_or_default()
    }
}

/// The lines of `notes` not placed, in order (`note_rest`).
pub fn note_rest(notes: &[PlacedNote]) -> NoteRest {
    notes.iter().flat_map(|n| n.rest().iter().copied()).collect()
}

/// A line of the flow: `item` indexes the flow, `line` the item's lines (a table's rows),
/// and `part`, inside a table row split across pages, each cell's first stacked line on
/// the page.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Position {
    /// The item.
    pub item: usize,
    /// Its line (a table's row).
    pub line: usize,
    /// A split row's part.
    pub part: Vec<u32>,
}

impl Position {
    /// Line `line` of item `item`.
    pub fn at(item: usize, line: usize) -> Position {
        Position {
            item,
            line,
            part: Vec::new(),
        }
    }
}

/// What a page is beyond its lines (`paginate.PageInfo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageInfo {
    /// Its number as `PAGE` shows it, before formatting.
    pub number: i64,
    /// The section of its first line.
    pub section: usize,
    /// Its section starts on it.
    pub section_first: bool,
    /// The header and footer kind it shows.
    pub story: StoryKind,
    /// A blank page Word inserts.
    pub blank: bool,
}

/// A line placed on a page: a paragraph's line with its top and pitch, exact px from the
/// page's top, and the column it is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlacedLine {
    /// The flow item.
    pub item: usize,
    /// The line's number in it.
    pub line: usize,
    /// The top of its pitch (below its space before).
    pub top: Px,
    /// Its pitch.
    pub pitch: Px,
    /// Where its paragraph's box starts (above its space before, past the space after of
    /// the paragraph before).
    pub paragraph_top: Px,
    /// The text column it is in, in a section of several columns.
    pub column: Option<usize>,
    /// That column's offset from the text area's left edge, twips.
    pub offset: i64,
}

/// A table's row (or part) placed on a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedTableRow {
    /// The flow item.
    pub item: usize,
    /// The row as placed.
    pub row: PlacedRow,
}

/// Why a page or column ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Why {
    /// It was full.
    Full,
    /// A column break.
    Column,
    /// A page break, or a section that starts a page.
    Page,
    /// A continuous section of other columns (the region).
    Region,
    /// The flow ended.
    End,
}

/// One page laid out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaidPage {
    /// Its 0-based place in the document, blank pages counted.
    pub page: usize,
    /// The number of the page before (the loop's state at its start).
    pub number_before: i64,
    /// Where it starts.
    pub start: Position,
    /// Where it ends (the next page's start).
    pub end: Position,
    /// The furthest position its decision read: an edit before it may change the page.
    pub reach: Position,
    /// It ended at a page or column break, or a section that starts a page.
    pub forced: bool,
    /// Its lines, in order.
    pub lines: Vec<PlacedLine>,
    /// Its tables' rows.
    pub rows: Vec<PlacedTableRow>,
    /// The lines of a note the page before could not hold, at its foot first.
    pub carried: NoteRest,
    /// The footnotes at its foot (none listed on a page of several columns, whose notes
    /// stand at its columns' feet).
    pub notes: Vec<PlacedNote>,
    /// The lines of a note it could not hold, for the next page's foot.
    pub carry_out: NoteRest,
    /// The rest of a note that a page of its own holds after this one (mode 15, a forced
    /// break ending this page).
    pub continued: Option<NoteRest>,
    /// A page the endnotes go on to: the continuation separator's line at its top, above
    /// its first line.
    pub opens: Option<OpeningLine>,
}

/// A line a page opens with that is no line of the flow: the endnotes' continuation
/// separator's, on a page they go on to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpeningLine {
    /// Its top, px from the page's top.
    pub top: Px,
    /// Its pitch.
    pub pitch: Px,
}

impl LaidPage {
    /// A page from `start` with nothing on it yet.
    pub fn empty(start: &Position, carried: NoteRest) -> LaidPage {
        LaidPage {
            page: 0,
            number_before: 0,
            start: start.clone(),
            end: start.clone(),
            reach: start.clone(),
            forced: false,
            lines: Vec::new(),
            rows: Vec::new(),
            carried,
            notes: Vec::new(),
            carry_out: Arc::from(Vec::new()),
            continued: None,
            opens: None,
        }
    }
}

/// Where the layout stopped: the flow item it could not lay out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    /// The item.
    pub item: usize,
    /// Its line (a table's row).
    pub line: usize,
    /// Why.
    pub reason: String,
}

/// The pages of a flow.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pagination {
    /// Each page laid out, in order.
    pub pages: Vec<LaidPage>,
    /// Every page, blank ones included, by its 0-based place.
    pub info: Vec<PageInfo>,
    /// Where the layout stopped, if it did.
    pub stopped: Option<Stopped>,
}

impl Pagination {
    /// How many pages there are, blank ones included.
    pub fn page_count(&self) -> usize {
        self.info.len()
    }

    /// Per page (0-based, blank pages counted) with footnotes at its foot: the lines a
    /// page before could not hold, and the notes placed on it.
    pub fn notes(&self) -> BTreeMap<usize, (NoteRest, Vec<PlacedNote>)> {
        let mut out = BTreeMap::new();
        for page in &self.pages {
            if !page.carried.is_empty() || !page.notes.is_empty() {
                out.insert(page.page, (Arc::clone(&page.carried), page.notes.clone()));
            }
            if let Some(rest) = &page.continued {
                out.insert(page.page + 1, (Arc::clone(rest), Vec::new()));
            }
        }
        out
    }
}

/// What laying out a page reads: the flow, the sections, the compatibility rules and the
/// stories' heights.
pub struct PageContext<'a> {
    /// The flow's items.
    pub items: &'a [FlowItem],
    /// The sections.
    pub sections: &'a Sections,
    /// The document is in compatibility mode 15 or later.
    pub mode15: bool,
    /// What Word does with a page ending in an automatic hyphen.
    pub hyphen_at_page_end: HyphenMove,
    /// The headers' and footers' heights.
    pub stories: &'a dyn StoryHeights,
    /// Per section, its footnote separator's and continuation separator's heights.
    pub separators: &'a BTreeMap<usize, (Px, Px)>,
    /// In mode 15, the footnotes of sections of several columns as flows of their own.
    pub note_items: &'a crate::flow::NoteItems,
}

/// What Word does with a page whose last line would end in an automatic hyphen
/// (`paginate.hyphen_at_page_end`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HyphenMove {
    /// Nothing: the page ends in the hyphen.
    Nothing,
    /// The line goes to the next page.
    Line,
    /// Only the hyphenated word does.
    Word,
}

/// The loop's state at a page's start: what a relayout goes on from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resume {
    /// The page's start.
    pub position: Position,
    /// Its 0-based place, blank pages counted.
    pub page: usize,
    /// The number of the page before.
    pub number: i64,
    /// The lines of a note the page before could not hold.
    pub carry: NoteRest,
}

/// Lays pages out from a start until the end, a stop, or until a caller says the rest is
/// known.
pub struct Paginator<'a> {
    cx: &'a PageContext<'a>,
}

impl<'a> Paginator<'a> {
    /// A paginator over `cx`.
    pub fn new(cx: &'a PageContext<'a>) -> Paginator<'a> {
        Paginator { cx }
    }

    /// Every page from `resume` (the flow's start where `None`) until the end or a stop,
    /// or until `until(state)` -- asked before every page after the first -- says the
    /// rest is known; returns the pages and the state it stopped at.
    pub fn run(&self, resume: Option<Resume>, until: &mut dyn FnMut(&Resume) -> bool) -> (Pagination, Option<Resume>) {
        let cx = self.cx;
        let mut out = Pagination::default();
        let Resume {
            mut position,
            mut page,
            mut number,
            mut carry,
        } = resume.unwrap_or(Resume {
            position: Position::default(),
            page: 0,
            number: 0,
            carry: Arc::from(Vec::new()),
        });
        let mut first = true;
        while position.item < cx.items.len() {
            let state = Resume {
                position: position.clone(),
                page,
                number,
                carry: Arc::clone(&carry),
            };
            if !first && until(&state) {
                return (out, Some(state));
            }
            first = false;
            let number_before = number;
            let info = self.page_info(&position, &mut page, &mut number, &mut out.info);
            out.info.push(info);
            let laid = cx.lay_page(&position, info.story, &carry);
            let mut laid = match laid {
                Ok(laid) => laid,
                Err((mut laid, stopped)) => {
                    laid.page = page;
                    laid.number_before = number_before;
                    out.pages.push(laid);
                    out.stopped = Some(stopped);
                    return (out, None);
                }
            };
            laid.page = page;
            laid.number_before = number_before;
            position = laid.end.clone();
            carry = Arc::clone(&laid.carry_out);
            page += 1;
            if !carry.is_empty() && laid.forced && cx.mode15 {
                // The rest of the note takes a page of its own before the forced break's.
                number += 1;
                out.info.push(PageInfo {
                    number,
                    section: info.section,
                    section_first: false,
                    story: cx.sections.story_kind(info.section, false, number),
                    blank: true,
                });
                laid.continued = Some(carry);
                carry = Arc::from(Vec::new());
                page += 1;
            }
            out.pages.push(laid);
        }
        (out, None)
    }

    /// The page starting at `position`: its number and story kind, any blank pages Word
    /// puts before it appended to `info` (moving `page` past them).
    fn page_info(&self, position: &Position, page: &mut usize, number: &mut i64, info: &mut Vec<PageInfo>) -> PageInfo {
        let cx = self.cx;
        let item = cx.items.get(position.item);
        let section = item.map_or(0, FlowItem::section);
        let before = position.item.checked_sub(1).and_then(|k| cx.items.get(k));
        let section_first =
            position.line == 0 && position.part.is_empty() && before.is_none_or(|b| b.section() != section);
        let start = cx.sections.get(section).and_then(|s| s.props.page_number_start);
        if section_first && position.item > 0 {
            let physical = i64::try_from(*page).unwrap_or(i64::MAX);
            let (n, blanks) = cx.sections.start_number(section, *number, physical);
            *number = n;
            for _ in 0..blanks {
                info.push(PageInfo {
                    number: n - 1,
                    section: before.map_or(section, FlowItem::section),
                    section_first: false,
                    story: StoryKind::Default,
                    blank: true,
                });
                *page += 1;
            }
        } else if let (true, Some(start)) = (section_first, start) {
            *number = start;
        } else {
            *number += 1;
        }
        PageInfo {
            number: *number,
            section,
            section_first,
            story: cx.sections.story_kind(section, section_first, *number),
            blank: false,
        }
    }
}

impl PageContext<'_> {
    /// One page from `start`: laid out, or laid out up to where the layout stops.
    #[allow(clippy::result_large_err)]
    fn lay_page(&self, start: &Position, story: StoryKind, carry: &NoteRest) -> Result<LaidPage, (LaidPage, Stopped)> {
        if self.sections.any_multi_column() {
            self.lay_columns_page(start, story, carry)
        } else {
            self.lay_one_column_page(start, story, carry)
        }
    }

    /// Section `section`'s footnote separator's and continuation separator's heights.
    fn separators(&self, section: usize) -> (Px, Px) {
        self.separators.get(&section).copied().unwrap_or((Px::ZERO, Px::ZERO))
    }

    /// Where the page's text starts and ends.
    fn edges(&self, section: usize, story: StoryKind) -> (Px, Px) {
        let (header, footer) = self.stories.heights(section, story);
        self.sections
            .get(section)
            .map_or((Px::ZERO, Px::ZERO), |s| s.edges(header, footer))
    }
}
