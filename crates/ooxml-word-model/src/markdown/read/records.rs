//! Owns: the records a reading of a story makes -- paragraphs (with their ids, style,
//! construct, list and inline items in the view), tables with their cells, and content
//! controls' starts and ends -- and where each element stands in its part ([`Placed`]).
//!
//! Must not: read the document; the reader ([`super::Reader`]) makes them.

use crate::docx::formatting::EffectiveRun;
use crate::docx::graphics::Graphic;
use crate::docx::numbering::Membership;
use crate::markdown::model::Inline;
use ooxml_xml::Element;
use std::sync::Arc;

/// An element of a part with its place.
#[derive(Debug, Clone)]
pub struct Placed {
    /// The element.
    pub element: Arc<Element>,
    /// Its place in its part.
    pub path: Box<[u32]>,
}

/// What a paragraph is in Markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A paragraph.
    Paragraph,
    /// A heading.
    Heading,
    /// A bullet list item.
    Bullet,
    /// A numbered list item.
    Ordered,
    /// A quote.
    Quote,
    /// A line of code.
    Code,
    /// A thematic break.
    Rule,
}

/// A paragraph as read.
#[derive(Debug, Clone)]
pub struct ParagraphRecord {
    /// Its id (the last of those joined).
    pub id: String,
    /// Its part.
    pub part: String,
    /// Its story.
    pub story: String,
    /// The paragraphs joined into it, in order.
    pub elements: Vec<Placed>,
    /// Its paragraph style's id.
    pub style_id: Option<String>,
    /// Its style's name.
    pub style_name: Option<String>,
    /// Its construct.
    pub kind: Kind,
    /// Its heading or list level.
    pub level: i64,
    /// Whether no rule of the style map names its style.
    pub unknown_style: bool,
    /// Its list.
    pub membership: Option<Membership>,
    /// Its list label.
    pub label: Option<String>,
    /// Its running number.
    pub number: Option<i64>,
    /// Its inline items.
    pub inline: Vec<Inline>,
    /// Its text in the view.
    pub text: String,
    /// The ids of the paragraphs joined before it.
    pub joins: Vec<String>,
    /// The notes it references.
    pub notes: Vec<String>,
    /// Its text boxes: the drawing's id, the box's content.
    pub text_boxes: Vec<(Option<String>, Placed)>,
    /// The section it ends: its id and type.
    pub section: Option<(String, String)>,
    /// Comments hoisted from the start of its line.
    pub hoisted: Vec<String>,
    /// What its own comment adds in the markup view.
    pub changes: Vec<String>,
    /// What `word_inspect` shows of it besides (runs, links, revisions...).
    pub detail: Detail,
}

/// A run of text as read: its text, character style's name, the properties it declares
/// itself, its effective formatting and the revision it is in.
#[derive(Debug, Clone)]
pub struct RunRecord {
    /// Its text in the view.
    pub text: String,
    /// Its character style's name (its id where the style is unknown).
    pub style: Option<String>,
    /// The local names of the properties it declares (`w:rStyle` aside).
    pub direct: Vec<String>,
    /// Its effective formatting.
    pub effective: EffectiveRun,
    /// The revision it is inside.
    pub revision: Option<String>,
}

/// A drawing as read: a picture or another drawing.
#[derive(Debug, Clone)]
pub struct DrawingRecord {
    /// Its id.
    pub id: Option<String>,
    /// Its kind (`picture` for a picture).
    pub kind: &'static str,
    /// `wp:docPr/@name`.
    pub name: Option<String>,
    /// `wp:docPr/@descr`.
    pub alt: String,
    /// Anchored (not inline).
    pub floating: bool,
    /// A picture's size in points.
    pub size: Option<(f64, f64)>,
    /// Whether it is a picture.
    pub picture: bool,
    /// The chart or SmartArt diagram it holds, where its parts could be read.
    pub graphic: Option<Graphic>,
    /// A group's members.
    pub members: Option<Vec<MemberRecord>>,
}

/// A drawing inside a group, as read.
#[derive(Debug, Clone)]
pub struct MemberRecord {
    /// Its id, `d:<group>/<cNvPr id>`.
    pub id: String,
    /// Its kind (`chart`, `diagram`, `group`, `picture`...).
    pub kind: &'static str,
    /// Its `cNvPr/@name`.
    pub name: Option<String>,
    /// The chart or SmartArt diagram it holds, where its parts could be read.
    pub graphic: Option<Graphic>,
}

/// What a paragraph holds besides its text, as `word_inspect` shows it.
#[derive(Debug, Clone, Default)]
pub struct Detail {
    /// Its runs of text.
    pub runs: Vec<RunRecord>,
    /// Its hyperlinks: id, target, text.
    pub links: Vec<(String, Option<String>, String)>,
    /// Its bookmarks shown (`bm:Name`).
    pub bookmarks: Vec<String>,
    /// The comments it references.
    pub comments: Vec<String>,
    /// The revisions in it (`None` for a record without an id).
    pub revisions: Vec<Option<String>>,
    /// Its pictures and drawings.
    pub drawings: Vec<DrawingRecord>,
    /// Its inline content controls: id, kind.
    pub controls: Vec<(Option<String>, &'static str)>,
    /// Its fields: instruction, cached result.
    pub fields: Vec<(String, String)>,
}

impl ParagraphRecord {
    /// The record a paragraph only counted leaves (never written).
    pub(super) fn counted(part: &str) -> ParagraphRecord {
        ParagraphRecord {
            id: String::new(),
            part: part.to_owned(),
            story: String::new(),
            elements: Vec::new(),
            style_id: None,
            style_name: None,
            kind: Kind::Paragraph,
            level: 0,
            unknown_style: false,
            membership: None,
            label: None,
            number: None,
            inline: Vec::new(),
            text: String::new(),
            joins: Vec::new(),
            notes: Vec::new(),
            text_boxes: Vec::new(),
            section: None,
            hoisted: Vec::new(),
            changes: Vec::new(),
            detail: Detail::default(),
        }
    }
}

/// A cell as read.
#[derive(Debug, Clone)]
pub struct CellRecord {
    /// Its id (`t:X/c<row>,<column>`).
    pub id: String,
    /// Its grid row and column.
    pub row: usize,
    /// Its grid column.
    pub column: usize,
    /// Rows it spans.
    pub row_span: usize,
    /// Columns it spans.
    pub column_span: usize,
    /// Its blocks.
    pub records: Vec<Record>,
    /// The `w:tc`'s place in its part.
    pub path: Box<[u32]>,
}

/// A table as read.
#[derive(Debug, Clone)]
pub struct TableRecord {
    /// Its id.
    pub id: String,
    /// The `w:tbl`.
    pub element: Arc<Element>,
    /// Its style's name.
    pub style_name: Option<String>,
    /// Its columns in the view.
    pub columns: usize,
    /// Its visible rows.
    pub rows: Vec<Vec<CellRecord>>,
    /// Header rows.
    pub header_rows: usize,
    /// Its innermost table style (`""`: none named), for its paragraphs' formatting.
    pub table_style: String,
    /// Its part.
    pub part: String,
    /// Its place there.
    pub path: Box<[u32]>,
    /// The ids of the paragraphs before it whose marks the view deletes, joined into its
    /// first cell (as a paragraph's [`ParagraphRecord::joins`]: Accept All takes them into
    /// the table).
    pub joins: Vec<String>,
}

/// A content control's start or end.
#[derive(Debug, Clone)]
pub struct Marker {
    /// Its id.
    pub id: String,
    /// Its kind.
    pub kind: &'static str,
    /// Whether it starts.
    pub begin: bool,
    /// Its tag.
    pub tag: Option<String>,
    /// Its title.
    pub alias: Option<String>,
    /// A start's: the ids of the paragraphs before the control whose marks the view
    /// deletes, joined into its first paragraph (as a paragraph's
    /// [`ParagraphRecord::joins`]).
    pub joins: Vec<String>,
}

/// A record.
#[derive(Debug, Clone)]
pub enum Record {
    /// A paragraph.
    Paragraph(Box<ParagraphRecord>),
    /// A table.
    Table(Box<TableRecord>),
    /// A content control's start or end.
    Marker(Marker),
}
