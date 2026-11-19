//! Owns: `word_format`'s definition (docx-agent's `tools/text.py`, `word_format`): its
//! arguments, their descriptions, bounds and intent rules, byte-equal to today's.
//!
//! Must not: run anything; the handler is `super::format`'s.

use ooxml_tools::{ArgEnum, ToolArgs};

/// Which matches a find item takes.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindExpect {
    /// Exactly one.
    One,
    /// Each one.
    All,
}

/// A paragraph's alignment.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    /// Left.
    Left,
    /// Centred.
    Center,
    /// Right.
    Right,
    /// Justified.
    Justify,
}

/// A direct property `clear` removes.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cleared {
    /// Bold.
    Bold,
    /// Italic.
    Italic,
    /// Underline.
    Underline,
    /// Strikethrough.
    Strike,
    /// Font size.
    Size,
    /// Font name.
    Font,
    /// Colour.
    Color,
    /// Highlight.
    Highlight,
    /// Alignment.
    Alignment,
    /// Space before.
    SpaceBefore,
    /// Space after.
    SpaceAfter,
    /// Line spacing.
    LineSpacing,
    /// Left indent.
    IndentLeft,
    /// Right indent.
    IndentRight,
    /// First-line indent.
    IndentFirst,
    /// Keep with next.
    KeepWithNext,
    /// Keep lines together.
    KeepLines,
    /// Page break before.
    PageBreakBefore,
}

/// Text to find and format (round 40).
#[derive(ToolArgs, Debug, Clone, PartialEq)]
pub struct FindItem {
    /// Text to format.
    pub text: String,
    /// Block or span to search.
    #[arg(refs, alias = "range")]
    pub within: Option<String>,
    /// Which match, from 1.
    #[arg(minimum = 1)]
    pub occurrence: Option<i64>,
    /// Default one: exactly one match.
    pub expect: Option<FindExpect>,
}

/// `word_format`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(name = "word_format", description = include_str!("word_format.md"), group = "word_style", mutates)]
pub struct WordFormat {
    /// Document id.
    pub doc: String,
    /// Paragraph, range (p:A@4:11), span (p:A..p:B) or table ids.
    #[arg(single_as_list, refs)]
    pub targets: Option<Vec<String>>,
    /// Or text to find and format.
    #[arg(single_as_list)]
    pub find: Option<Vec<FindItem>>,
    /// A paragraph to look like: style, list and level, indents, keeps.
    #[arg(refs)]
    pub like: Option<String>,
    /// Paragraph style name.
    pub paragraph_style: Option<String>,
    /// Character style name.
    pub character_style: Option<String>,
    /// Bold.
    pub bold: Option<bool>,
    /// Italic.
    pub italic: Option<bool>,
    /// Underline.
    pub underline: Option<bool>,
    /// Strikethrough.
    pub strike: Option<bool>,
    /// Font size.
    #[arg(minimum = 1, maximum = 1638)]
    pub size: Option<f64>,
    /// Font name.
    pub font: Option<String>,
    /// Theme colour or RRGGBB.
    pub color: Option<String>,
    /// yellow, green... or none.
    pub highlight: Option<String>,
    /// Alignment.
    pub alignment: Option<Alignment>,
    /// Before the paragraph.
    #[arg(minimum = 0, maximum = 1584, alias = "top")]
    pub space_before: Option<f64>,
    /// After the paragraph.
    #[arg(minimum = 0, maximum = 1584, alias = "bottom")]
    pub space_after: Option<f64>,
    /// Times single, e.g. 1.15.
    #[arg(minimum = 0.06, maximum = 132)]
    pub line_spacing: Option<f64>,
    /// Left indent.
    #[arg(minimum = -1584, maximum = 1584)]
    pub indent_left: Option<f64>,
    /// Right indent.
    #[arg(minimum = -1584, maximum = 1584)]
    pub indent_right: Option<f64>,
    /// First line; negative hangs.
    #[arg(minimum = -1584, maximum = 1584)]
    pub indent_first: Option<f64>,
    /// Keep with next.
    pub keep_with_next: Option<bool>,
    /// Keep lines together.
    pub keep_lines: Option<bool>,
    /// Page break before.
    pub page_break_before: Option<bool>,
    /// Keep each target whole on one page: a span's blocks, a table, a paragraph's lines.
    pub keep_together: Option<bool>,
    /// First remove direct formatting, keeping styles. Default false.
    pub clear_direct: Option<bool>,
    /// Direct properties to remove, back to the style's.
    pub clear: Option<Vec<Cleared>>,
}
