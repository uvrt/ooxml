//! Owns: the extension point tables are laid out through ([`TableLayout`], [`TableFlow`]):
//! what the paginator asks of a top-level table -- its rows, which of them keep with the
//! next, its header rows, its edges against the items around it, and one page's part of
//! it placed from a row ([`TableFlow::place`], docx2svg's `table.place_table`) -- and what
//! the pages ask of it afterwards: the lines each placed row draws ([`TableFlow::lines`])
//! and the paragraphs each row's cells stack ([`TableFlow::stacks`]).
//!
//! Must not: lay a table out. [`crate::tables::WordTables`] implements [`TableLayout`];
//! [`NoTables`] refuses every table, and a layout through it stops at the first one.

use crate::faces::Faces;
use crate::paragraph::MeasuredParagraph;
use crate::px::Px;
use crate::sections::SectionGeometry;
use ooxml_base::{Locale, Q};
use ooxml_text::Measurer;
use ooxml_word_model::model::SectionProps;
use ooxml_word_model::model::{Paragraph, Table};
use ooxml_word_model::{Model, WordDocument};
use std::fmt::Debug;
use std::sync::Arc;

/// Where one page's part of a table is placed from, and against what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRequest {
    /// Where its first row's top border starts.
    pub y: Px,
    /// Where the page's (or column's) text ends.
    pub bottom: Px,
    /// The first row to place.
    pub row: usize,
    /// A row split across the page before: each cell's first stacked line on this page.
    pub part: Vec<u32>,
    /// Nothing is on the page above it.
    pub first_on_page: bool,
    /// The document is in compatibility mode 15 or later.
    pub mode15: bool,
    /// A row that keeps with the next is split where it does not fit (below mode 15, when
    /// keeping it whole would leave the page empty).
    pub split_kept: bool,
    /// Only the rows before this one: the page's part of them, and this row next.
    pub stop: Option<usize>,
}

/// A cell merged down whose merge goes across a page: its lines on this page, drawn from
/// the top of the row they are carried on, in the merge's column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carried {
    /// The merge's first row.
    pub row: usize,
    /// Its cell in that row.
    pub cell: usize,
    /// Its first stacked line here.
    pub start: u32,
    /// One past its last stacked line here.
    pub end: u32,
}

/// One row (or a split row's part) placed on the page (docx2svg's `RowPiece`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedRow {
    /// The row.
    pub row: usize,
    /// A header row repeated at the page's top.
    pub header: bool,
    /// Where its top border starts.
    pub top: Px,
    /// Its top border's height.
    pub top_border: Px,
    /// Its height below its top border, down to the edge below it.
    pub height: Px,
    /// Each cell's first stacked line on this page.
    pub start: Vec<u32>,
    /// Each cell's line after its last stacked line on this page.
    pub end: Vec<u32>,
    /// The row began on an earlier page.
    pub continued: bool,
    /// The row goes on to the next page.
    pub split: bool,
    /// Merges down going across the page, drawn from this row's top.
    pub carry: Vec<Carried>,
    /// Not one line of it fits on an empty page: placed whole, past the page's foot.
    pub overflowed: bool,
}

impl PlacedRow {
    /// Where on the page it starts, as a [`crate::paginate::Position`]'s part: each cell's
    /// first stacked line where the row began on an earlier page, else none.
    pub fn part(&self) -> Vec<u32> {
        if self.continued {
            self.start.clone()
        } else {
            Vec::new()
        }
    }

    /// Where the stack goes on below it.
    pub fn bottom(&self) -> Px {
        self.top + self.top_border + self.height
    }
}

/// One page's part of a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TablePage {
    /// Where the stack goes on below it.
    pub end: Px,
    /// The row (and a split row's part) the next page starts with; `None`: the table
    /// ended on this page.
    pub next: Option<(usize, Vec<u32>)>,
    /// The rows placed, in order.
    pub rows: Vec<PlacedRow>,
}

impl TablePage {
    /// Whether a row that is not a repeated header is on the page.
    pub fn any_body(&self) -> bool {
        self.rows.iter().any(|r| !r.header)
    }
}

/// One line a placed row draws: a cell paragraph's line (a nested table's included), at
/// its top and pitch, exact px from the page's top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellLine {
    /// Its paragraph: its index among the table's paragraphs in document order (nested
    /// tables' included), from the table's first.
    pub block: usize,
    /// Its line in the paragraph.
    pub line: usize,
    /// The top of its pitch.
    pub top: Px,
    /// Its pitch.
    pub pitch: Px,
    /// Whether it draws anything (a line of nothing but spaces before the last does not).
    pub drawn: bool,
}

/// One stacked line of a cell, by its paragraph (as [`CellLine::block`]); `None` for one
/// that draws nothing or is not a paragraph's (a nested table's).
pub type StackedBlock = Option<usize>;

/// Why a table cannot be laid out.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("table not laid out: {0}")]
pub struct Unsupported(pub String);

/// Twips text keeps off a floating table at least, whatever `*FromText` says
/// (`MIN_FROM_TEXT`, `make_float_table_probe.py`, `edge`, every mode).
pub const MIN_FROM_TEXT: i64 = 10;

/// Where a floating table is positioned (docx2svg's `table.Floating`): what it is
/// positioned against vertically (`text`, `margin` or `page`), by `y` twips or aligned
/// (`y_spec`), and how far text keeps off each side, twips. Across, it is laid out in its
/// frame (the column, or the page) as a table indented `x` twips, or aligned `x_spec`, is
/// in the column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Floating {
    /// `vertAnchor`: `text`, `margin` or `page`.
    pub vertical: String,
    /// `tblpY`.
    pub y: i64,
    /// `tblpYSpec`.
    pub y_spec: Option<String>,
    /// `leftFromText`.
    pub left: i64,
    /// `rightFromText`.
    pub right: i64,
    /// `topFromText`.
    pub top: i64,
    /// `bottomFromText`.
    pub bottom: i64,
    /// `horzAnchor="page"`: laid out across the page, not the column.
    pub page: bool,
    /// `tblpX`.
    pub x: i64,
    /// `tblpXSpec`.
    pub x_spec: Option<String>,
}

impl Floating {
    /// Its top, exact px from the page's (`floating_top`): `paragraph_top` is the top of
    /// the paragraph after it, `height` the table's, borders included.
    pub fn top(&self, section: &SectionProps, paragraph_top: Px, height: Px) -> Px {
        if self.vertical == "text" {
            return paragraph_top + Px::twips(self.y);
        }
        let m = &section.margins;
        let page_height = Px::twips(section.page_size.height);
        let (start, extent) = if self.vertical == "margin" {
            let start = Px::twips(m.top.abs());
            (start, page_height - start - Px::twips(m.bottom.abs()))
        } else {
            (Px::ZERO, page_height)
        };
        match self.y_spec.as_deref() {
            Some("center") => start + Px::from((extent - height).value() / Q::from_integer(2)),
            Some("bottom") => start + extent - height,
            _ => start + Px::twips(self.y),
        }
    }
}

/// A floating table as text goes beside it: where it is positioned, its outer grid lines
/// (exact px from the page's left edge) and how far its box reaches past them, twips
/// (`floating_edges`: its outer borders' outer halves, or 15 twips where it has none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloatingTable {
    /// Where it is positioned.
    pub position: Floating,
    /// Its first grid line.
    pub first: Q,
    /// Its last grid line.
    pub last: Q,
    /// How far its box reaches left of its first grid line and right of its last.
    pub edges: (Q, Q),
}

/// A top-level table measured for the page stack.
pub trait TableFlow: Debug + Send + Sync {
    /// How many rows it has.
    fn rows(&self) -> usize;
    /// Whether row `row` keeps with the next.
    fn keeps_with_next(&self, row: usize) -> bool;
    /// How many rows at its top repeat as headers.
    fn header_rows(&self) -> usize;
    /// Whether its first cell starts with a block of paragraph style `style` (as written:
    /// `None` no `w:pStyle`; a nested table has none): the contextual spacing of the
    /// paragraph before it.
    fn opens_with_style(&self, style: Option<&str>) -> bool;
    /// Its top border's height (the first row's).
    fn top_border(&self) -> Px;
    /// Its bottom border's height.
    fn bottom_border(&self) -> Px;
    /// One page's part of it.
    fn place(&self, request: &TableRequest) -> Result<TablePage, Unsupported>;
    /// The lines `row`, placed on a page, draws, in drawing order.
    fn lines(&self, row: &PlacedRow) -> Vec<CellLine>;
    /// Row `row`'s cells' stacked lines, cell by cell.
    fn stacks(&self, row: usize) -> Vec<Vec<StackedBlock>>;
    /// The measure of its paragraph `block` (as [`CellLine::block`]), if it is one.
    fn paragraph(&self, block: usize) -> Option<Arc<MeasuredParagraph>>;
    /// The table as its layout's own type, for what reads more of it than the paginator
    /// (the pages' drawing, the differential).
    fn as_any(&self) -> &dyn std::any::Any;
    /// Where it is positioned, where it floats (`None`: it is laid out in the flow).
    fn floating(&self) -> Option<&FloatingTable> {
        None
    }
    /// Whether, laid out in a room `width` twips wide, it keeps within it (`table_fits`):
    /// in mode 15 its right border's outer edge, below mode 15 its last cell's text.
    fn fits(&self, width: Q) -> bool;
}

/// A table laid out again in another room than its column: beside drawings, or in a
/// column right of its section's first (`TableItem.room`). Rooms are equal by where they
/// are: the table in a room is a function of the table and the room.
#[derive(Debug, Clone)]
pub struct TableRoom {
    /// The room's left edge, twips right of the column's.
    pub left: i64,
    /// Its width, twips.
    pub width: i64,
    /// The table laid out in it.
    pub flow: Arc<dyn TableFlow>,
}

impl PartialEq for TableRoom {
    fn eq(&self, other: &TableRoom) -> bool {
        self.left == other.left && self.width == other.width
    }
}

impl Eq for TableRoom {}

/// Lays a flow's tables out again in other rooms while its pages are laid out.
pub trait TableRooms {
    /// Flow item `item`'s table laid out in a column `left` twips right of its own and
    /// `width` twips wide; `None` where it cannot be.
    fn room(&self, item: usize, left: i64, width: i64) -> Option<TableRoom>;
}

/// What laying a table out reads and keeps: the document and its model, the faces, the
/// locale and the measures kept by content.
pub struct TableContext<'a, 'm> {
    /// The document.
    pub document: &'a WordDocument,
    /// Its model (resolution is memoised there).
    pub model: &'m mut Model,
    /// Measures kept by content.
    pub measurer: &'m mut Measurer,
    /// The faces.
    pub faces: &'a dyn Faces,
    /// The locale measures are keyed under.
    pub locale: &'a Locale,
}

/// One top-level table to lay out.
#[derive(Debug, Clone, Copy)]
pub struct TableJob<'a> {
    /// The table.
    pub table: &'a Table,
    /// Its section.
    pub section: &'a SectionGeometry,
    /// The paragraph before it, if one is.
    pub previous: Option<&'a Paragraph>,
    /// Its paragraphs' list labels, in document order (nested tables' included).
    pub labels: &'a [Option<String>],
}

/// Lays top-level tables out.
pub trait TableLayout: Send + Sync {
    /// The table measured for the page stack.
    fn flow_table(&self, cx: &mut TableContext<'_, '_>, job: &TableJob<'_>) -> Result<Arc<dyn TableFlow>, Unsupported>;
}

/// No table layout: every table is refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTables;

impl TableLayout for NoTables {
    fn flow_table(
        &self,
        _cx: &mut TableContext<'_, '_>,
        _job: &TableJob<'_>,
    ) -> Result<Arc<dyn TableFlow>, Unsupported> {
        Err(Unsupported("no table layout is configured".into()))
    }
}
