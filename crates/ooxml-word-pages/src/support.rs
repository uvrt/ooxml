//! Owns: what the Rust layout does not lay out or draw as docx2svg does yet, found in a
//! version before or after it is laid out -- the lanes that close each case are named
//! beside it (L3e: tables beside drawings, floating tables, drawings in cells, STYLEREF;
//! L3f: footnotes in columns and beneath the text, line numbers, column separators, text
//! beside a drop cap in columns). Two kinds:
//!
//! - **layout** ([`unsupported`]): the pages or the facts would differ; the caller lays the
//!   whole document out another way (docx2svg's Python layout, frozen for this);
//! - **paint** ([`unpaintable`]): the layout and its facts are docx2svg's, but some pages'
//!   SVG is not yet; a caller drawing such a page draws it another way.
//!
//! Each reason is one of [`REASONS`], a stable label for a counter. The tests are
//! conservative: a document is sent back where a case might apply, never laid out wrong.
//!
//! Must not: lay out, draw, or guess past what the model says.

use crate::error::Fallback;
use ooxml_word_layout::DocumentLayout;
use ooxml_word_model::model::{Block, Graphic, InlineObject, Paragraph, Table};
use ooxml_word_model::value::Value;
use ooxml_word_model::WordDocument;

/// A floating table (`w:tblpPr`).
pub const FLOATING_TABLE: &str = "floating table";
/// A floating drawing that text wraps around, in a table cell (docx2svg's `wrap_cell`):
/// the layout does not lay its cell out yet.
pub const WRAP_IN_CELL: &str = "drawing text wraps around in a table cell";
/// Another floating drawing, or a drawing that is not a picture, in a table cell.
pub const DRAWING_IN_CELL: &str = "drawing in a table cell";
/// A drawing text wraps beside (`wrapSquare`, `wrapTight`, `wrapThrough`): the lines'
/// segments beside it are the paint's from L3f.
pub const TEXT_BESIDE_DRAWING: &str = "text beside a drawing";
/// A drawing holding a text box: its lines are its painter's (until P3), so neither its
/// facts nor its paint are the layout's.
pub const TEXT_BOX: &str = "text box";
/// A `STYLEREF` field.
pub const STYLEREF: &str = "STYLEREF field";
/// Footnotes referenced in a section of several columns.
pub const FOOTNOTES_IN_COLUMNS: &str = "footnotes in columns";
/// Footnotes placed beneath the text (`w:pos="beneathText"`).
pub const FOOTNOTES_BENEATH_TEXT: &str = "footnotes beneath the text";
/// A body the layout gives no page (docx2svg draws one blank page).
pub const NO_PAGES: &str = "no pages";
/// The layout stopped at a block it cannot lay out.
pub const LAYOUT_STOPPED: &str = "layout stopped";
/// A header or footer it cannot measure.
pub const STORY_UNSUPPORTED: &str = "header or footer not measured";
/// Line numbers (`w:lnNumType`): drawn by the paint only from L3f.
pub const LINE_NUMBERS: &str = "line numbers";
/// A line between columns (`w:cols/@w:sep`).
pub const COLUMN_SEPARATOR: &str = "column separator";
/// A frame other than a drop cap (`w:framePr`): docx2svg lays it out by an approximate
/// rule and says so in its coverage, which the Rust layout does not.
pub const FRAME: &str = "frame";
/// A drop cap: the text beside it is the paint's from L3f.
pub const DROP_CAP: &str = "drop cap";
/// Endnotes going on to a page under the continuation separator, which the paint draws
/// from L3f.
pub const ENDNOTES_CONTINUED: &str = "endnotes continued";
/// A drawing a painter of DrawingML, charts or SmartArt fills (`DrawingSlot`), where none
/// is given.
pub const DRAWINGS: &str = "drawing for a painter";
/// Something else the page's paint lists as not drawn.
pub const UNPAINTED: &str = "not painted";

/// Every reason, for a counter's labels.
pub const REASONS: &[&str] = &[
    FLOATING_TABLE,
    WRAP_IN_CELL,
    DRAWING_IN_CELL,
    TEXT_BESIDE_DRAWING,
    TEXT_BOX,
    STYLEREF,
    FOOTNOTES_IN_COLUMNS,
    FOOTNOTES_BENEATH_TEXT,
    NO_PAGES,
    LAYOUT_STOPPED,
    STORY_UNSUPPORTED,
    LINE_NUMBERS,
    COLUMN_SEPARATOR,
    FRAME,
    DROP_CAP,
    ENDNOTES_CONTINUED,
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

/// Why a version cannot be laid out by the Rust layout as docx2svg lays it out, before it
/// is: the first case found, in the body, its headers and footers, and its notes.
pub fn unsupported(document: &WordDocument) -> Option<Fallback> {
    let multi = document.sections.iter().any(|s| s.properties.column_count > 1);
    for section in document.sections.iter() {
        if section.properties.footnotes.position.as_deref() == Some("beneathText") {
            return Some(fallback(FOOTNOTES_BENEATH_TEXT, "w:sectPr"));
        }
    }
    let mut scan = Scan {
        multi_column: multi,
        found: None,
    };
    scan.story(&document.body, false);
    for blocks in document.stories.values() {
        scan.story(blocks, false);
    }
    for blocks in document
        .footnotes
        .notes
        .values()
        .chain(document.endnotes.notes.values())
    {
        scan.story(blocks, false);
    }
    scan.found
}

/// Why a version laid out cannot be taken as docx2svg's: the layout stopped, a story was
/// not measured, or endnotes go on under the continuation separator.
pub fn stopped(layout: &DocumentLayout) -> Option<Fallback> {
    if layout.page_count() == 0 {
        return Some(fallback(NO_PAGES, "w:body"));
    }
    if let Some(stop) = &layout.pagination.stopped {
        return Some(fallback(LAYOUT_STOPPED, &stop.reason));
    }
    if let Some(why) = layout.stories.unsupported.first() {
        return Some(fallback(STORY_UNSUPPORTED, why));
    }
    layout
        .pagination
        .pages
        .iter()
        .find(|p| p.opens.is_some())
        .map(|p| fallback(ENDNOTES_CONTINUED, &format!("page {}", p.page + 1)))
}

/// Why some pages of a version laid out are not drawn as docx2svg draws them, where every
/// page is concerned: the sections' line numbers and column separators. (A page's own
/// reasons are its paint's `unpainted`.)
pub fn unpaintable(document: &WordDocument) -> Option<Fallback> {
    for section in document.sections.iter() {
        if section.properties.line_numbering.is_some() {
            return Some(fallback(LINE_NUMBERS, "w:lnNumType"));
        }
        if section.properties.column_separator && section.properties.column_count > 1 {
            return Some(fallback(COLUMN_SEPARATOR, "w:cols"));
        }
    }
    None
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

/// Whether a drawing (or a member of its group) holds a text box.
fn text_box(graphic: &Graphic) -> bool {
    !graphic.text.is_empty() || graphic.children.iter().any(|g| text_box(g))
}

struct Scan {
    multi_column: bool,
    found: Option<Fallback>,
}

impl Scan {
    fn story(&mut self, blocks: &[Block], in_cell: bool) {
        for block in blocks {
            if self.found.is_some() {
                return;
            }
            match block {
                Block::Paragraph(p) => self.paragraph(p, in_cell),
                Block::Table(t) => self.table(t),
            }
        }
    }

    fn table(&mut self, table: &Table) {
        if table.properties.contains_key("tblpPr") {
            self.found = Some(fallback(FLOATING_TABLE, &table.path));
            return;
        }
        for row in table.rows.iter() {
            for cell in row.iter() {
                self.story(cell, true);
            }
        }
    }

    fn paragraph(&mut self, paragraph: &Paragraph, in_cell: bool) {
        match frame(paragraph) {
            Some(true) => {
                self.found = Some(fallback(DROP_CAP, &paragraph.path));
                return;
            }
            Some(false) => {
                self.found = Some(fallback(FRAME, &paragraph.path));
                return;
            }
            None => {}
        }
        for run in paragraph.runs.iter() {
            if run
                .anchors
                .iter()
                .any(|a| a.wrap.as_deref().is_some_and(|w| SIDE_WRAPS.contains(&w)))
            {
                let reason = if in_cell { WRAP_IN_CELL } else { TEXT_BESIDE_DRAWING };
                self.found = Some(fallback(reason, &paragraph.path));
                return;
            }
            if run
                .field
                .as_ref()
                .is_some_and(|f| f.keyword.eq_ignore_ascii_case("STYLEREF"))
            {
                self.found = Some(fallback(STYLEREF, &paragraph.path));
                return;
            }
            if self.multi_column && run.breaks.iter().any(|(_, mark)| mark.starts_with("footnoteReference")) {
                self.found = Some(fallback(FOOTNOTES_IN_COLUMNS, &paragraph.path));
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
            if in_cell {
                let graphic = run
                    .drawings
                    .iter()
                    .any(|d| matches!(d, InlineObject::Graphic(g) if &*g.kind != "picture"));
                if graphic || !run.anchors.is_empty() {
                    self.found = Some(fallback(DRAWING_IN_CELL, &paragraph.path));
                    return;
                }
            }
        }
    }
}
