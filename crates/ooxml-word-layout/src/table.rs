//! Owns: the extension point tables are laid out through ([`TableLayout`], [`TableFlow`]):
//! what the paginator asks of a top-level table -- its rows, which of them keep with the
//! next, its header rows, its edges against the items around it, and one page's part of
//! it placed from a row ([`TableFlow::place`], docx2svg's `table.place_table`).
//!
//! Must not: lay a table out. Lane L3b implements [`TableLayout`]; until then
//! [`NoTables`] refuses every table, and the layout stops at the first one.

use crate::px::Px;
use crate::sections::SectionGeometry;
use ooxml_word_model::model::{Paragraph, Table};
use ooxml_word_model::value::Text;
use ooxml_word_model::WordDocument;
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
}

/// One row (or a split row's part) placed on the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedRow {
    /// The row.
    pub row: usize,
    /// A header row repeated at the page's top.
    pub header: bool,
    /// Where it starts.
    pub top: Px,
    /// Its height, its top border included.
    pub height: Px,
    /// A split row's part: each cell's first stacked line on this page.
    pub part: Vec<u32>,
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

/// Why a table cannot be laid out.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("table not laid out: {0}")]
pub struct Unsupported(pub String);

/// A top-level table measured for the page stack.
pub trait TableFlow: Debug + Send + Sync {
    /// How many rows it has.
    fn rows(&self) -> usize;
    /// Whether row `row` keeps with the next.
    fn keeps_with_next(&self, row: usize) -> bool;
    /// How many rows at its top repeat as headers.
    fn header_rows(&self) -> usize;
    /// Its first paragraph's style (for the space after of a contextual paragraph before).
    fn first_style(&self) -> Option<Text>;
    /// Its top border's height (the first row's).
    fn top_border(&self) -> Px;
    /// Its bottom border's height.
    fn bottom_border(&self) -> Px;
    /// One page's part of it.
    fn place(&self, request: &TableRequest) -> Result<TablePage, Unsupported>;
}

/// Lays top-level tables out.
pub trait TableLayout: Send + Sync {
    /// The table measured for the page stack in `section`; `previous` the paragraph before
    /// it, if one is; `labels` its cells' paragraphs' list labels, in document order.
    fn flow_table(
        &self,
        document: &WordDocument,
        table: &Table,
        section: &SectionGeometry,
        previous: Option<&Paragraph>,
        labels: &[Option<String>],
    ) -> Result<Arc<dyn TableFlow>, Unsupported>;
}

/// No table layout: every table is refused (lane L3b provides one).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTables;

impl TableLayout for NoTables {
    fn flow_table(
        &self,
        _document: &WordDocument,
        _table: &Table,
        _section: &SectionGeometry,
        _previous: Option<&Paragraph>,
        _labels: &[Option<String>],
    ) -> Result<Arc<dyn TableFlow>, Unsupported> {
        Err(Unsupported("no table layout is configured".into()))
    }
}
