//! Owns: what the Rust layout does not lay out or draw as docx2svg does yet, found in a
//! version before or after it is laid out. Two kinds:
//!
//! - **layout** ([`unsupported`]): the pages or the facts would differ; the caller lays the
//!   whole document out another way (docx2svg's Python layout, frozen for this);
//! - **paint** (a page's paint's own `unpainted`, and drawings for their painters): the
//!   layout and its facts are docx2svg's, but that page's SVG is not yet; a caller drawing
//!   such a page draws it another way.
//!
//! Each reason is one of [`REASONS`], a stable label for a counter. The tests are
//! conservative: a document is sent back where a case might apply, never laid out wrong.
//!
//! Must not: lay out, draw, or guess past what the model says.

use crate::error::Fallback;
use ooxml_word_layout::geometry::text_box;
use ooxml_word_layout::DocumentLayout;
use ooxml_word_model::model::{Block, Graphic, InlineObject, Paragraph, Table};
use ooxml_word_model::value::Value;
use ooxml_word_model::WordDocument;

/// A drawing text wraps beside (`wrapSquare`, `wrapTight`, `wrapThrough`) in a header, a
/// footer or a note: docx2svg stacks such a paragraph of a header or footer as an
/// estimated stand-in, which the Rust layout does not draw.
pub const TEXT_BESIDE_DRAWING: &str = "text beside a drawing";
/// A text box holding what the layout does not lay out in one: a floating drawing text
/// wraps around (docx2svg stops the box's text there), or an inline drawing holding text
/// of its own (docx2svg lays it out on the box's scratch page).
pub const TEXT_BOX: &str = "text box";
/// A `STYLEREF` field in a footnote or an endnote (the body's and the headers' and
/// footers' are laid out).
pub const STYLEREF: &str = "STYLEREF field";
/// The layout stopped at a block it cannot lay out.
pub const LAYOUT_STOPPED: &str = "layout stopped";
/// A header or footer it cannot measure.
pub const STORY_UNSUPPORTED: &str = "header or footer not measured";
/// A frame other than a drop cap (`w:framePr`) in a header, a footer or a note (one in the
/// body or a cell is laid out as a paragraph where it stands, as docx2svg lays it out).
pub const FRAME: &str = "frame";
/// A drop cap anywhere but among the body's top-level paragraphs.
pub const DROP_CAP: &str = "drop cap";
/// A drawing a painter of DrawingML, charts or SmartArt fills (`DrawingSlot`), where none
/// is given.
pub const DRAWINGS: &str = "drawing for a painter";
/// Something else the page's paint lists as not drawn.
pub const UNPAINTED: &str = "not painted";

/// Every reason, for a counter's labels.
pub const REASONS: &[&str] = &[
    TEXT_BESIDE_DRAWING,
    TEXT_BOX,
    STYLEREF,
    LAYOUT_STOPPED,
    STORY_UNSUPPORTED,
    FRAME,
    DROP_CAP,
    DRAWINGS,
    UNPAINTED,
];

/// The wraps a drawing puts text beside.
const SIDE_WRAPS: &[&str] = &["wrapSquare", "wrapTight", "wrapThrough"];

fn fallback(reason: &'static str, at: &str) -> Fallback {
    Fallback {
        reason,
        at: at.to_owned(),
    }
}

/// Where a scan is: the body's top-level blocks, a table cell, a header or footer, or a
/// note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Story {
    Body,
    Cell,
    HeaderFooter,
    Note,
}

/// Why a version cannot be laid out by the Rust layout as docx2svg lays it out, before it
/// is: the first case found, in the body, its headers and footers, and its notes.
pub fn unsupported(document: &WordDocument) -> Option<Fallback> {
    let mut scan = Scan { found: None };
    scan.story(&document.body, Story::Body);
    for blocks in document.stories.values() {
        scan.story(blocks, Story::HeaderFooter);
    }
    for blocks in document
        .footnotes
        .notes
        .values()
        .chain(document.endnotes.notes.values())
    {
        scan.story(blocks, Story::Note);
    }
    scan.found
}

/// Why a version laid out cannot be taken as docx2svg's: the layout stopped, or a story
/// was not measured.
pub fn stopped(layout: &DocumentLayout) -> Option<Fallback> {
    if let Some(stop) = &layout.pagination.stopped {
        return Some(fallback(LAYOUT_STOPPED, &stop.reason));
    }
    layout
        .stories
        .unsupported
        .first()
        .map(|why| fallback(STORY_UNSUPPORTED, why))
}

/// Whether a paragraph is framed (`w:framePr` declared): `Some(true)` as a drop cap.
fn frame(paragraph: &Paragraph) -> Option<bool> {
    let props = paragraph.properties.as_ref()?;
    match props.declared.get("framePr") {
        Some(Value::Attrs(attrs)) => Some(
            attrs
                .iter()
                .any(|(k, v)| &**k == "dropCap" && matches!(&**v, "drop" | "margin")),
        ),
        _ => None,
    }
}

/// Whether a drawing (or a member of its group) holds a text box the layout does not lay
/// out.
fn text_box(graphic: &Graphic) -> bool {
    text_box::unsupported_in(&graphic.text).is_some() || graphic.children.iter().any(|g| text_box(g))
}

struct Scan {
    found: Option<Fallback>,
}

impl Scan {
    fn story(&mut self, blocks: &[Block], story: Story) {
        for block in blocks {
            if self.found.is_some() {
                return;
            }
            match block {
                Block::Paragraph(p) => self.paragraph(p, story),
                Block::Table(t) => self.table(t),
            }
        }
    }

    fn table(&mut self, table: &Table) {
        for row in table.rows.iter() {
            for cell in row.iter() {
                self.story(cell, Story::Cell);
            }
        }
    }

    fn paragraph(&mut self, paragraph: &Paragraph, story: Story) {
        match frame(paragraph) {
            Some(true) if story != Story::Body => {
                self.found = Some(fallback(DROP_CAP, &paragraph.path));
                return;
            }
            Some(false) if matches!(story, Story::HeaderFooter | Story::Note) => {
                self.found = Some(fallback(FRAME, &paragraph.path));
                return;
            }
            _ => {}
        }
        for run in paragraph.runs.iter() {
            if matches!(story, Story::HeaderFooter | Story::Note)
                && run
                    .anchors
                    .iter()
                    .any(|a| a.wrap.as_deref().is_some_and(|w| SIDE_WRAPS.contains(&w)))
            {
                self.found = Some(fallback(TEXT_BESIDE_DRAWING, &paragraph.path));
                return;
            }
            if story == Story::Note
                && run
                    .field
                    .as_ref()
                    .is_some_and(|f| f.keyword.eq_ignore_ascii_case("STYLEREF"))
            {
                self.found = Some(fallback(STYLEREF, &paragraph.path));
                return;
            }
            let boxed = run.anchors.iter().filter_map(|a| a.graphic.as_deref()).any(text_box)
                || run
                    .drawings
                    .iter()
                    .any(|d| matches!(d, InlineObject::Graphic(g) if text_box(g)));
            if boxed {
                self.found = Some(fallback(TEXT_BOX, &paragraph.path));
                return;
            }
        }
    }
}
