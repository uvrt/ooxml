//! Owns: `word_format_table`'s definition (docx-agent's `tools/objects.py`,
//! `word_format_table`): its arguments, byte-equal to today's.
//!
//! Must not: change the document.

use ooxml_tools::{ArgEnum, ToolArgs};

/// What `word_format_table` formats.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The table.
    Table,
    /// A row.
    Row,
    /// A column.
    Column,
    /// A cell.
    Cell,
}

/// What a table's percent width is of.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthOf {
    /// The text width.
    Text,
    /// The body's paragraphs' lines beside the table.
    Body,
    /// Word's AutoFit to Window.
    Window,
}

/// Left, centre or right.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Across {
    /// Left.
    Left,
    /// Centred.
    Center,
    /// Right.
    Right,
}

/// Every border's style.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    /// No border.
    None,
    /// A single line.
    Single,
    /// A double line.
    Double,
    /// A thick line.
    Thick,
    /// Dots.
    Dotted,
    /// Dashes.
    Dashed,
}

/// A cell's text position.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vertical {
    /// At the top.
    Top,
    /// Centred.
    Center,
    /// At the bottom.
    Bottom,
}

/// A row's height rule.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeightRule {
    /// At least the height.
    AtLeast,
    /// Exactly the height.
    Exact,
}

/// A table style option, as Table Design names it.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// Header row.
    HeaderRow,
    /// First column.
    FirstColumn,
    /// Banded rows.
    BandedRows,
    /// Total row.
    TotalRow,
    /// Last column.
    LastColumn,
    /// Banded columns.
    BandedColumns,
}

/// `word_format_table`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(
    name = "word_format_table",
    description = include_str!("word_format_table.md"),
    group = "word_objects",
    mutates
)]
pub struct WordFormatTable {
    /// Document id.
    pub doc: String,
    /// Table (t:...).
    #[arg(refs)]
    pub table: String,
    /// What to format.
    pub scope: Scope,
    /// row, cell: from 0.
    #[arg(minimum = 0)]
    pub row: Option<i64>,
    /// column, cell: from 0.
    #[arg(minimum = 0)]
    pub column: Option<i64>,
    /// table: style name.
    pub style: Option<String>,
    /// table, column, cell: width.
    #[arg(minimum = 1, maximum = 1584)]
    pub width: Option<f64>,
    /// table: width, % of width_of.
    #[arg(minimum = 1, maximum = 100)]
    pub width_percent: Option<f64>,
    /// Default text; body: paragraphs' lines; window: AutoFit Window.
    pub width_of: Option<WidthOf>,
    /// table: on the page.
    pub alignment: Option<Across>,
    /// Fill RRGGBB, or none.
    pub shading: Option<String>,
    /// Every border's style, inside ones too.
    pub borders: Option<BorderStyle>,
    /// cell, row, column: text position.
    pub vertical_alignment: Option<Vertical>,
    /// row: height.
    #[arg(minimum = 1, maximum = 1584)]
    pub height: Option<f64>,
    /// row. Default at_least.
    pub height_rule: Option<HeightRule>,
    /// table: rows repeated on each page.
    #[arg(minimum = 0, maximum = 500)]
    pub header_rows: Option<i64>,
    /// table: style options on, the rest off.
    pub look: Option<Vec<Look>>,
    /// row (table: all): never split across pages.
    pub cant_split: Option<bool>,
    /// table: on one page; rows keep with next, last row not.
    pub keep_together: Option<bool>,
    /// table kept together: last row keeps with next block too.
    pub keep_with_next: Option<bool>,
}
