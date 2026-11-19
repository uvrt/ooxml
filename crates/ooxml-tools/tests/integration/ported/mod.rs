//! Three of today's tools as Rust definitions, for the schema equality test: Word's
//! `word_set_text` and `word_format`, and the shared `replace_text` (its kind rules
//! included). Their handlers stay in Python; lane P0d wires tools to the session.
//!
//! The golden exports beside them (`tests/golden/*.json`) are what these definitions
//! export; `python/docx-agent/tests/test_rust_tool_specs.py` checks those against today's
//! Python definitions byte for byte and runs the Python checker on them.
#![allow(dead_code)] // the handlers that would read the fields come with the format crates

use ooxml_tools::{ArgEnum, IntOrStr, ToolArgs};

/// One paragraph's new text.
#[derive(ToolArgs, Debug, PartialEq)]
pub struct SetTextItem {
    /// Paragraph, or table cell t:X/c<row>,<col>.
    #[arg(refs)]
    pub target: String,
    /// The whole new text; \n starts a paragraph; **strong**, *emphasis*, [text]{Character style}.
    #[arg(facts)]
    pub text: String,
    /// Paragraph style for it (name or id).
    pub paragraph_style: Option<String>,
}

/// `word_set_text`.
#[derive(ToolArgs, Debug, PartialEq)]
#[tool(name = "word_set_text", description = include_str!("word_set_text.md"), mutates)]
pub struct WordSetText {
    /// Document id.
    pub doc: String,
    /// Paragraph, or table cell t:X/c<row>,<col>.
    #[arg(refs)]
    pub target: Option<String>,
    /// The whole new text, as items' text.
    #[arg(facts)]
    pub text: Option<String>,
    /// Its paragraph style.
    pub paragraph_style: Option<String>,
    /// Or several paragraphs.
    pub items: Option<Vec<SetTextItem>>,
}

/// How many matches a call expects.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// Exactly one.
    One,
    /// Every one.
    All,
}

/// Text to find and format.
#[derive(ToolArgs, Debug, PartialEq)]
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
    pub expect: Option<Expect>,
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

/// A direct property to remove.
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
    /// Font.
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

/// `word_format`.
#[derive(ToolArgs, Debug, PartialEq)]
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

/// Which stories a search covers.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stories {
    /// The body.
    Body,
    /// Every story.
    All,
    /// A deck's layouts.
    Layouts,
    /// A deck's masters.
    Masters,
}

/// `replace_text`, shared by every format.
#[derive(ToolArgs, Debug, PartialEq)]
#[tool(name = "replace_text", description = include_str!("replace_text.md"), mutates)]
pub struct ReplaceText {
    /// Document id.
    pub doc: String,
    /// Text to replace.
    pub find: String,
    /// Replacement; with regex, \1 is a group.
    #[arg(facts)]
    pub replace: String,
    /// Matches expected.
    #[arg(default = "one")]
    pub expect: Expect,
    /// Decks: slide numbers from 1, or ids (s:256, $ref). Default all.
    #[arg(kinds("pptx"), refs)]
    pub slides: Option<Vec<IntOrStr<1>>>,
    /// Documents: only within this block or span (p:A..t:B).
    #[arg(kinds("docx"))]
    pub range: Option<String>,
    /// Documents: body (default) or all stories. Decks: body is the slides; layouts, masters, or all.
    #[arg(
        kinds("pptx", "docx"),
        kind_value("layouts", "pptx"),
        kind_value("masters", "pptx"),
        kind_description("pptx", "Body (default): the slides; layouts, masters, or all."),
        kind_description("docx", "Body (default) or all stories.")
    )]
    pub stories: Option<Stories>,
    /// find is a Python regex. Default false.
    pub regex: Option<bool>,
}

/// Every ported tool's spec, with the kinds its Python handlers serve today.
pub fn ported() -> Vec<(ooxml_tools::ToolSpec, Vec<String>, &'static [&'static str])> {
    vec![
        (WordSetText::spec(), WordSetText::problems(), &["docx"]),
        (WordFormat::spec(), WordFormat::problems(), &["docx"]),
        (ReplaceText::spec(), ReplaceText::problems(), &["docx"]),
    ]
}
