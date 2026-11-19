//! Owns: `word_styles`' definition (docx-agent's `tools/style.py`, `word_styles`): its
//! arguments, their descriptions, bounds and nested shapes (a list level, tab stops, a
//! table style's regions and their borders), byte-equal to today's.
//!
//! Must not: run anything; the handler is `super::styles`'.

use super::format_args::Alignment;
use ooxml_tools::{ArgEnum, ToolArgs};

/// What `word_styles` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum StylesAction {
    /// The styles.
    List,
    /// One style.
    Describe,
    /// A new style.
    Add,
    /// A style changed.
    Modify,
    /// A style removed.
    Remove,
    /// The unused ones removed.
    PurgeUnused,
    /// The table style new tables get.
    SetDefault,
    /// The document's language.
    SetLanguage,
}

/// A style's kind.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleKind {
    /// Paragraph.
    Paragraph,
    /// Character.
    Character,
    /// Table.
    Table,
}

/// A tab stop's alignment.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabAlignment {
    /// Left.
    Left,
    /// Centred.
    Center,
    /// Right.
    Right,
    /// On the decimal point.
    Decimal,
    /// A bar.
    Bar,
    /// An inherited stop cleared.
    Clear,
}

/// A tab stop's leader.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabLeader {
    /// None.
    None,
    /// Dots.
    Dot,
    /// Hyphens.
    Hyphen,
    /// Underscores.
    Underscore,
    /// A heavy line.
    Heavy,
    /// Middle dots.
    MiddleDot,
}

/// A table style's region.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionName {
    /// The style's own.
    WholeTable,
    /// The header row.
    HeaderRow,
    /// The total row.
    TotalRow,
    /// The first column.
    FirstColumn,
    /// The last column.
    LastColumn,
    /// Odd banded rows.
    BandedRows,
    /// Even banded rows.
    EvenBandedRows,
    /// Odd banded columns.
    BandedColumns,
    /// Even banded columns.
    EvenBandedColumns,
    /// The top left cell.
    TopLeftCell,
    /// The top right cell.
    TopRightCell,
    /// The bottom left cell.
    BottomLeftCell,
    /// The bottom right cell.
    BottomRightCell,
}

/// A region border's style.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionBorderStyle {
    /// None stated (over a lower region's).
    None,
    /// Single.
    Single,
    /// Double.
    Double,
    /// Thick.
    Thick,
    /// Dotted.
    Dotted,
    /// Dashed.
    Dashed,
}

/// A border's side.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionSide {
    /// Top.
    Top,
    /// Bottom.
    Bottom,
    /// Left.
    Left,
    /// Right.
    Right,
    /// Between rows.
    InsideH,
    /// Between cells.
    InsideV,
}

/// `modify`'s list level.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
pub struct StyleNumbering {
    /// A list's number, a style in it, or none to unlink.
    pub list: String,
    /// From 0. Default 0.
    #[arg(minimum = 0, maximum = 8)]
    pub level: Option<i64>,
}

/// One tab stop.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
pub struct TabItem {
    /// Points. Omit: the text column's end.
    #[arg(minimum = -1584, maximum = 1584)]
    pub position: Option<f64>,
    /// Default left; clear: an inherited stop.
    pub alignment: Option<TabAlignment>,
    /// Default none.
    pub leader: Option<TabLeader>,
}

/// A region's borders.
// Today's schema describes `style` not at all: a doc comment would become a description.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, PartialEq)]
pub struct RegionBordersArg {
    pub style: RegionBorderStyle,
    /// Default all; inside_*: between its cells.
    pub sides: Option<Vec<RegionSide>>,
    /// RRGGBB.
    pub color: Option<String>,
    /// Points. Default 0.5.
    #[arg(minimum = 0.25, maximum = 6)]
    pub width: Option<f64>,
}

/// One region's formatting.
// Today's schema describes `bold`, `italic` and `alignment` not at all.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, PartialEq)]
pub struct RegionItem {
    /// Where. Columns: first, last, banded only.
    pub region: RegionName,
    /// Fill: RRGGBB, theme colour, or none.
    pub shading: Option<String>,
    /// Text colour.
    pub color: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    /// Text size.
    #[arg(minimum = 1, maximum = 1638)]
    pub size: Option<f64>,
    pub alignment: Option<Alignment>,
    /// Sides' borders; other styles: repeat the region.
    pub borders: Option<RegionBordersArg>,
    /// Banded: rows/columns per band.
    #[arg(minimum = 1, maximum = 100)]
    pub band_size: Option<i64>,
    /// Remove its formatting first.
    pub clear: Option<bool>,
}

/// `word_styles`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(name = "word_styles", description = include_str!("word_styles.md"), group = "word_style", mutates)]
pub struct WordStyles {
    /// Document id.
    pub doc: String,
    /// What to do.
    pub action: StylesAction,
    /// Style name.
    pub name: Option<String>,
    /// Default paragraph.
    pub kind: Option<StyleKind>,
    /// add: based on this style.
    pub based_on: Option<String>,
    /// add: style of the next paragraph.
    pub next_style: Option<String>,
    /// remove: style its users get instead.
    pub replacement: Option<String>,
    /// set_language: a tag such as fr-BE, nl-BE, en-US.
    pub language: Option<String>,
    /// list: only styles in use.
    pub in_use_only: Option<bool>,
    /// modify: link to a list level.
    pub numbering: Option<StyleNumbering>,
    /// add, modify: the style's tab stops, replacing its own; [] removes them.
    pub tabs: Option<Vec<TabItem>>,
    /// Table style, add/modify: formatting per region (whole_table: the style's own).
    pub regions: Option<Vec<RegionItem>>,
    /// Bold.
    pub bold: Option<bool>,
    /// Italic.
    pub italic: Option<bool>,
    /// Font size.
    #[arg(minimum = 1, maximum = 1638)]
    pub size: Option<f64>,
    /// Font name.
    pub font: Option<String>,
    /// Theme colour or RRGGBB.
    pub color: Option<String>,
    /// Alignment.
    pub alignment: Option<Alignment>,
    /// Before the paragraph.
    #[arg(minimum = 0, maximum = 1584)]
    pub space_before: Option<f64>,
    /// After the paragraph.
    #[arg(minimum = 0, maximum = 1584)]
    pub space_after: Option<f64>,
    /// Times single, e.g. 1.15.
    #[arg(minimum = 0.06, maximum = 132)]
    pub line_spacing: Option<f64>,
    /// Left indent.
    #[arg(minimum = -1584, maximum = 1584)]
    pub indent_left: Option<f64>,
    /// First line; negative hangs.
    #[arg(minimum = -1584, maximum = 1584)]
    pub indent_first: Option<f64>,
    /// Keep with next.
    pub keep_with_next: Option<bool>,
    /// Fill: RRGGBB, theme colour, 'accent1 lighter 80%' or none.
    pub shading: Option<String>,
}
