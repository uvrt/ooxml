//! Owns: a page's paint (docs/architecture/README.md, section 8: geometry, then paint):
//! what docx2svg's page walk puts on a page -- every line's glyph runs at their pen
//! positions ([`Span`], `layout.Span`), the rectangles the text needs (decorations,
//! paragraph rules, table borders and shading: [`Rule`]), and where the drawings go
//! ([`Drawn`]: inline pictures and drawings, by their extents, for a painter of drawings
//! to fill) -- and the page as SVG through `ooxml-scene`'s exact display list ([`svg`]),
//! byte for byte what docx2svg writes.
//!
//! The drawings are the seam with the painters of DrawingML, charts and SmartArt: a
//! [`Drawn`] says what is drawn where, by its part and relationship; it is written as
//! docx2svg writes a picture it cannot carry until a painter fills it.
//!
//! Must not: place lines (the geometry has), measure (the text engine has), or read a
//! drawing's or picture's bytes.

mod body_keys;
mod cell_floats;
mod decorate;
pub mod drawings;
pub mod feed;
mod line_numbers;
pub mod names;
mod page_stories;
mod paragraphs;
mod positions;
mod separators;
mod spans;
pub mod svg;
mod tables;
pub mod text_box;

pub use crate::naming::ItemName;
pub use body_keys::body_line_keys;
pub use line_numbers::{LineNumber, LineNumbers};
pub use page_stories::paint_stories;
pub use positions::{line_positions, Positions, Setting, TabSpan};
pub(crate) use separators::section_holders;

use crate::engine::DocumentLayout;
use crate::faces::Faces;
use crate::flow::{body_paragraphs, FlowItem};
use crate::geometry::stories::{PageParts, StoryPage};
use crate::geometry::{DrawnLine, LineSource, PageGeometry};
use crate::naming::paragraph_item;
use crate::paginate::LaidPage;
use crate::paragraph::MeasuredParagraph;
use crate::px::{CompactPx, Px};
use crate::table::PlacedRow;
use ooxml_word_model::ids::Sub;
use ooxml_word_model::model::{Block, Paragraph};
use ooxml_word_model::value::Text;
use ooxml_word_model::{Model, WordDocument};
use std::sync::Arc;

/// What a span of glyphs is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpanKind {
    /// A run's text.
    Text,
    /// A list label's.
    Label,
    /// The hyphen a soft hyphen draws at a line's end.
    Hyphen,
    /// A tab's leader.
    Leader,
    /// The space an underlined tab is underlined under.
    Tab,
    /// A line's number (`w:lnNumType`).
    LineNumber,
}

impl SpanKind {
    /// Its name as the SVG says it (`data-docx-kind`).
    pub fn name(self) -> &'static str {
        match self {
            SpanKind::Text => "text",
            SpanKind::Label => "label",
            SpanKind::Hyphen => "hyphen",
            SpanKind::Leader => "leader",
            SpanKind::Tab => "tab",
            SpanKind::LineNumber => "line-number",
        }
    }
}

/// Consecutive glyphs of one run, in one format, on one line (`layout.Span`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// What it is drawn for: its paragraph's run (by its place among the projection's
    /// runs), label or the paragraph itself; a path for a paragraph no story's index holds
    /// (a text box's).
    pub name: ItemName,
    /// Its characters.
    pub chars: Vec<char>,
    /// Each glyph's pen x, px from the page's left edge, kept compact ([`Span::x`] reads
    /// one as [`Px`]).
    pub xs: Vec<CompactPx>,
    /// The baseline it is drawn on.
    pub y: i64,
    /// The pen after its last glyph.
    pub end: Px,
    /// Its face.
    pub face: Text,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// The size it is drawn at, half points.
    pub half_points: i64,
    /// Its colour (`RRGGBB`), `None` automatic.
    pub color: Option<String>,
    /// `w:u`'s value, where underlined.
    pub underline: Option<String>,
    /// `w:strike`.
    pub strike: bool,
    /// `w:dstrike`.
    pub double_strike: bool,
    /// The highlight's colour.
    pub highlight: Option<&'static str>,
    /// The run's shading fill.
    pub shading: Option<String>,
    /// What it is.
    pub kind: SpanKind,
    /// The line's baseline.
    pub line_baseline: i64,
    /// `w:vertAlign` of the glyph that opened it, where a script (`Span.vertical_align`).
    pub vertical_align: Option<Text>,
    /// Its run's Word 2010 text effects, where it states any (`Span.paint`).
    pub effects: Option<TextEffects>,
}

/// A run's Word 2010 text effects (`w14:textFill`, `w14:textOutline`): DrawingML a painter
/// of drawings paints the span's glyphs with (the SVG asks [`svg::Drawings::text_paint`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEffects {
    /// `w14:textFill`.
    pub fill: Option<ooxml_word_model::value::TextEffect>,
    /// `w14:textOutline`.
    pub outline: Option<ooxml_word_model::value::TextEffect>,
}

impl TextEffects {
    /// The effects run properties `props` state, where they state any.
    pub fn of(props: &ooxml_word_model::value::Props) -> Option<TextEffects> {
        let effect = |name: &str| match props.get(name) {
            Some(ooxml_word_model::value::Value::TextEffect(e)) => Some(e.clone()),
            _ => None,
        };
        let (fill, outline) = (effect("textFill"), effect("textOutline"));
        (fill.is_some() || outline.is_some()).then_some(TextEffects { fill, outline })
    }
}

/// A filled rectangle the text needs (`layout.Rule`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// What it is: `underline`, `strike`, `highlight`, `shading`, `paragraph-border`,
    /// `paragraph-shading`, `table-border`, `cell-shading`, `separator`.
    pub kind: &'static str,
    /// What it is drawn for: a block, a table's cell, a section's columns; a path for what
    /// no story's index holds.
    pub name: ItemName,
    /// Its left edge, px.
    pub x: Px,
    /// Its top.
    pub y: Px,
    /// Its width.
    pub width: Px,
    /// Its height.
    pub height: Px,
    /// Its colour, `None` automatic (black).
    pub color: Option<String>,
    /// A border's or underline's style (`single`, `double`, `dotted`...; `Rule.style`).
    pub style: Option<String>,
}

/// A drawing a page draws, by its extent: what a painter of pictures, DrawingML, charts
/// or SmartArt fills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawn {
    /// Its run (a picture bullet: its paragraph's label); a path for a text box's.
    pub name: ItemName,
    /// The relationship of its picture part, where it is a picture.
    pub relationship: Option<Text>,
    /// The part the relationship is the part's own (a header's), `None` the document's.
    pub part: Option<Text>,
    /// Its left edge, px.
    pub x: Px,
    /// Its top.
    pub y: Px,
    /// Its width.
    pub width: Px,
    /// Its height.
    pub height: Px,
    /// Its picture's `a:srcRect` crop (left, top, right, bottom fractions), where cropped.
    pub crop: Option<[ooxml_base::Q; 4]>,
}

impl Span {
    /// Glyph `k`'s pen x.
    pub fn x(&self, k: usize) -> Option<Px> {
        self.xs.get(k).map(CompactPx::get)
    }

    /// Every glyph's pen x.
    pub fn pens(&self) -> impl Iterator<Item = Px> + '_ {
        self.xs.iter().map(CompactPx::get)
    }
}

/// One line drawn, with its spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaintedLine {
    /// Its paragraph: its key, or a path for a paragraph no story's index holds.
    pub name: ItemName,
    /// Its paragraph's runs, as projected: what its spans' anchors name a run by.
    pub runs: Arc<[ooxml_word_model::model::Run]>,
    /// Its paragraph's `w14:paraId`.
    pub paragraph_id: Option<Text>,
    /// Its number in the paragraph.
    pub number: usize,
    /// Its baseline.
    pub baseline: i64,
    /// The exact top of its pitch, px from the page's top (`Line.top`).
    pub top: Px,
    /// Its pitch (`Line.pitch`).
    pub pitch: Px,
    /// Its text column, in a section of several columns (`Line.column`).
    pub column: Option<usize>,
    /// The device-pixel rows its text fills (`Line.band`).
    pub band: (i64, i64),
    /// Its spans.
    pub spans: Vec<Span>,
    /// A header's or footer's line: its kind and part.
    pub story: Option<(&'static str, Text)>,
}

/// What a page paints, in the order docx2svg's walk adds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagePaint {
    /// The page's geometry.
    pub geometry: PageGeometry,
    /// Its lines.
    pub lines: Vec<PaintedLine>,
    /// Its rules.
    pub rules: Vec<Rule>,
    /// Its inline pictures.
    pub pictures: Vec<Drawn>,
    /// Its drawings, for their painters: inline shapes, groups, charts and diagrams, and
    /// floating drawings, in the order met.
    pub drawings: Vec<drawings::DrawingSlot>,
    /// What the page holds that this paint does not draw yet, by reason: the page's SVG is
    /// not complete where any is listed.
    pub unpainted: Vec<String>,
    /// The floating drawings positioned against a frame the model does not know: not
    /// drawn (docx2svg's `drawing-position-unknown`), by path.
    pub unplaced: Vec<String>,
    /// Its drawings' text boxes painted, by drawing (none until their painter has found
    /// them: [`text_box::paint_text_box`]).
    pub text_boxes: Vec<text_box::TextBoxPaint>,
}

pub use ooxml_fonts::DrawingName;

/// Where the paint reads how faces are named: the port to the machine's fonts.
pub trait FaceNames: Send + Sync {
    /// How the SVG names `family` in `bold` and `italic`; `None` where the document's name
    /// and style already reach the face.
    fn drawing_name(&self, family: &str, bold: bool, italic: bool) -> Option<DrawingName>;
}

/// Every face named as the document names it.
#[derive(Debug, Clone, Copy, Default)]
pub struct DocumentNames;

impl FaceNames for DocumentNames {
    fn drawing_name(&self, _family: &str, _bold: bool, _italic: bool) -> Option<DrawingName> {
        None
    }
}

/// What glyph outlines are scaled to (`GLYPH_SIZES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlyphSize {
    /// The size rounded to whole device px, as Word draws its ink.
    #[default]
    Device,
    /// The size itself.
    Exact,
}

/// What painting reads beyond the layout: the faces and how they are named, the glyph
/// size, and the decimal separator a decimal tab lines up.
pub struct PaintContext<'a> {
    /// The faces (metrics and decorations).
    pub faces: &'a dyn Faces,
    /// How faces are named in the SVG.
    pub names: &'a dyn FaceNames,
    /// The glyph size.
    pub glyph_size: GlyphSize,
    /// The decimal separator.
    pub decimal: char,
}

/// Lines to paint and what they are drawn from: the body's flow on a laid page, or a
/// header's or footer's flow.
struct LineSet<'a> {
    /// The flow the lines' sources index.
    items: &'a [FlowItem],
    /// Its top-level blocks.
    blocks: &'a [Block],
    /// Its paragraphs, cells included: the body's those its layout keeps (one walk of the
    /// body, not one per page painted), a story's or text box's its own.
    paragraphs: std::borrow::Cow<'a, [Arc<Paragraph>]>,
    /// The page that laid the body's lines out (its paragraphs broken again there).
    laid: Option<&'a LaidPage>,
    /// Its paragraphs broken again where they were placed (the page's, or a note area's).
    broken: Option<&'a crate::paginate::LineStore>,
    /// The lines.
    lines: &'a [DrawnLine],
    /// Its tables' rows: the flow item, the row, and the room it was laid out in.
    rows: Vec<(usize, &'a PlacedRow, Option<&'a crate::table::TableRoom>)>,
    /// A story's kind and part.
    story: Option<(&'static str, Text)>,
    /// The page's frame, where the set's floating drawings are placed (the body's, a
    /// header's or a footer's; not the footnotes').
    frame: Option<&'a crate::geometry::PageFrame>,
    /// The right edge a note separator's line is cut at.
    clip: Option<Px>,
    /// The page (0-based).
    page: usize,
    /// A walked story's paragraphs' tops for their drawings (`Walked::paragraph_tops`).
    paragraph_tops: &'a [(usize, Px)],
    /// The layout.
    layout: &'a DocumentLayout,
}

/// A line's floating drawings, placed (`_anchors`): a body line's on its laid page, a
/// header's or footer's where its walk put its paragraph.
fn floats(
    set: &LineSet<'_>,
    (frame, item): (&crate::geometry::PageFrame, usize),
    (paragraph, resolved, measured): (&Paragraph, &ooxml_word_model::ResolvedParagraph, &MeasuredParagraph),
    line: &DrawnLine,
    out: &mut spans::PageOut,
) {
    let own = || {
        set.items
            .get(item)
            .and_then(FlowItem::para)
            .map_or(&[][..], |p| p.anchors.as_slice())
    };
    let (anchors, paragraph_top, (bands, wraps), page) = match set.laid {
        Some(laid) => (
            laid.broken
                .get(&item)
                .map_or_else(own, |again| again.anchors.as_slice()),
            laid.lines
                .iter()
                .find(|l| l.item == item && l.line == line.number)
                .map(|l| l.paragraph_top),
            (laid.bands.as_slice(), laid.wraps.as_slice()),
            laid.page,
        ),
        None => (
            own(),
            set.paragraph_tops.iter().find(|(i, _)| *i == item).map(|(_, top)| *top),
            (&[][..], &[][..]),
            set.page,
        ),
    };
    if anchors.is_empty() {
        return;
    }
    let job = drawings::FloatJob {
        layout: set.layout,
        frame,
        page,
        paragraph_top,
        bands,
        wraps,
        paragraph,
        resolved,
        measured,
        anchors,
        item,
        line,
    };
    let (slots, missed) = drawings::line_floats(&job, &mut out.drawn);
    out.drawings.extend(slots);
    out.unplaced.extend(missed);
}

/// The paragraph a drawn line is of, and its measure on the page.
fn paragraph_of(set: &LineSet<'_>, line: &DrawnLine) -> Option<(Arc<Paragraph>, Arc<MeasuredParagraph>)> {
    match line.source {
        LineSource::Flow { item } => {
            let p = set.items.get(item)?.para()?;
            let measured = match set.broken.and_then(|broken| broken.get(&item)) {
                Some(again) => Arc::clone(&again.measured),
                None => Arc::clone(&p.measured),
            };
            Some((Arc::clone(set.paragraphs.get(line.block?)?), measured))
        }
        LineSource::Cell { item, block } => {
            let Some(FlowItem::Table(t)) = set.items.get(item) else {
                return None;
            };
            Some((Arc::clone(set.paragraphs.get(line.block?)?), t.flow.paragraph(block)?))
        }
        LineSource::DropCap { item } => {
            let drop = set.items.get(item)?.para()?.drop_cap.clone()?;
            match set.blocks.get(drop.body) {
                Some(Block::Paragraph(p)) => Some((Arc::clone(p), Arc::clone(&drop.measured))),
                _ => None,
            }
        }
        LineSource::Opening { .. } => None,
    }
}

/// What painting a set of lines writes to.
struct PaintOut<'o> {
    out: &'o mut spans::PageOut,
    lines: &'o mut Vec<PaintedLine>,
}

/// A paragraph's borders and shading on the page, after its last line there
/// (`_paragraph_rules`); line `last` of the set is its last there.
fn paragraph_rules(
    set: &LineSet<'_>,
    model: &mut Model,
    document: &WordDocument,
    (last, item, width): (usize, usize, Px),
    out: &mut spans::PageOut,
) {
    let Some(p) = set.items.get(item).and_then(FlowItem::para) else {
        return;
    };
    let mine: Vec<&DrawnLine> = set
        .lines
        .get(..=last)
        .unwrap_or_default()
        .iter()
        .rev()
        .take_while(|l| l.source == LineSource::Flow { item })
        .collect();
    let (Some(first), Some(end)) = (mine.last(), mine.first()) else {
        return;
    };
    let mut resolve = |k: Option<usize>| match k.and_then(|k| set.blocks.get(k)) {
        Some(Block::Paragraph(q)) => Some(model.resolve_paragraph(document, q).paragraph),
        _ => None,
    };
    let Some(pp) = resolve(Some(p.body)) else { return };
    let previous = resolve(p.body.checked_sub(1));
    let following = resolve(Some(p.body + 1));
    let (before_same, after_same) = crate::borders::box_neighbours(&pp, previous.as_deref(), following.as_deref());
    let starts_here = first.number == 0;
    let ends_here = end.number + 1 == p.count();
    let (left, right) = paragraphs::text_edges(&pp, (first.left, first.left + width));
    let paragraph = set.paragraphs.get(p.block);
    let bx = paragraphs::ParagraphBox {
        pp: &pp,
        name: paragraph.map_or(ItemName::Path(Text::default()), |q| paragraph_item(q, Sub::Whole)),
        left,
        right,
        band: (first.band.0, end.band.1),
        y_gap: first.top - p.top_border,
        y_after: end.top + end.pitch + if ends_here { p.border } else { Px::ZERO },
        top_side: starts_here && !before_same,
        between_above: starts_here && before_same,
        bottom_side: ends_here && !after_same,
    };
    let (borders, shaded) = paragraphs::paragraph_rects(&bx);
    if let Some(s) = shaded {
        out.rules.insert(0, s);
    }
    out.rules.extend(borders);
}

/// A set's tables' borders and shading, row by row (a row's nested tables' first): each
/// run of rows as the table was laid out for them -- in their room, where they were laid
/// out in one (beside drawings, in a column right of the first), else the table's own.
fn table_rules(set: &LineSet<'_>, out: &mut spans::PageOut) {
    let mut rules = tables::TableRules::default();
    let mut k = 0;
    while let Some(&(item, _, room)) = set.rows.get(k) {
        let run: Vec<&PlacedRow> = set
            .rows
            .get(k..)
            .unwrap_or_default()
            .iter()
            .take_while(|(i, _, r)| *i == item && *r == room)
            .map(|(_, r, _)| *r)
            .collect();
        k += run.len();
        let Some(FlowItem::Table(t)) = set.items.get(item) else {
            continue;
        };
        let flow = room.map_or(&t.flow, |r| &r.flow);
        let Some(table) = flow.as_any().downcast_ref::<crate::tables::MeasuredTable>() else {
            out.unpainted.push("a table of another layout".into());
            continue;
        };
        rows_rules(table, &run, &mut rules);
    }
    for s in rules.shading {
        out.rules.insert(0, s);
    }
    out.rules.extend(rules.borders);
}

/// A table's rows on a page, in order, each after its nested tables' (`_table_drawn`).
fn rows_rules(table: &crate::tables::MeasuredTable, rows: &[&PlacedRow], out: &mut tables::TableRules) {
    for (k, piece) in rows.iter().enumerate() {
        for (nested, top) in tables::nested_tables(table, piece) {
            let placed = tables::whole_rows(nested, top);
            let refs: Vec<&PlacedRow> = placed.iter().collect();
            rows_rules(nested, &refs, out);
        }
        let bottom = piece.top + piece.top_border + piece.height;
        let parted = rows.get(k + 1).is_some_and(|next| next.top != bottom);
        let last = k + 1 == rows.len() || parted || piece.row + 1 == table.rows.len();
        tables::row_rules(table, piece, bottom, last, out);
    }
}

/// The table laid out for flow item `item` of a set (the table's own layout, not a room's).
fn cell_table<'s>(set: &LineSet<'s>, item: usize) -> Option<&'s crate::tables::MeasuredTable> {
    match set.items.get(item) {
        Some(FlowItem::Table(t)) => t.flow.as_any().downcast_ref::<crate::tables::MeasuredTable>(),
        _ => None,
    }
}

/// The cell lines of a set's tables holding floating drawings, by flow item.
fn anchored_lines<'s>(set: &LineSet<'s>) -> Vec<(usize, crate::tables::AnchoredLine<'s>)> {
    let mut out = Vec::new();
    for (item, row, room) in &set.rows {
        let table = match room {
            Some(room) => room.flow.as_any().downcast_ref::<crate::tables::MeasuredTable>(),
            None => cell_table(set, *item),
        };
        if let Some(table) = table {
            out.extend(table.anchored_lines(row, 0).into_iter().map(|a| (*item, a)));
        }
    }
    out
}

/// Paint a set of lines: each line's spans, each paragraph's rules after its last line,
/// the tables' rules after the lines.
fn paint_set(
    set: &LineSet<'_>,
    model: &mut Model,
    document: &WordDocument,
    (cx, width): (&PaintContext<'_>, &dyn Fn(Option<usize>) -> Px),
    to: PaintOut<'_>,
) {
    let pictures_from = to.out.pictures.len();
    let anchored = anchored_lines(set);
    for (k, line) in set.lines.iter().enumerate() {
        if let LineSource::Opening { .. } = line.source {
            // Painted with the page's opening (`PageParts::opening`), before the body.
            continue;
        }
        let Some((paragraph, measured)) = paragraph_of(set, line) else {
            to.out.unpainted.push(format!("{:?}", line.source));
            continue;
        };
        let resolved = model.resolve_paragraph(document, &paragraph);
        let extra = match (set.broken, line.source) {
            (Some(broken), LineSource::Flow { item }) => broken.get(&item).and_then(|r| r.extra.get(line.number)),
            (_, LineSource::Cell { item, block }) => {
                cell_table(set, item).and_then(|t| t.line_extra(block, line.number))
            }
            _ => None,
        };
        let job = spans::LineJob {
            document,
            paragraph: &paragraph,
            resolved: &resolved,
            measured: &measured,
            line,
            extra,
            clip: set.clip,
        };
        let from = to.out.drawings.len();
        if let Some(mut painted) = spans::paint_line(cx, &job, to.out) {
            painted.story = set.story.clone();
            to.lines.push(painted);
        }
        if let (LineSource::Flow { item }, Some(frame)) = (line.source, set.frame) {
            floats(set, (frame, item), (&paragraph, &resolved, &measured), line, to.out);
        }
        if let (LineSource::Cell { item, block }, Some(frame)) = (line.source, set.frame) {
            let found = anchored
                .iter()
                .find(|(i, a)| *i == item && a.block == block && a.line == line.number && a.top == line.top);
            if let Some((_, anchored)) = found {
                let job = cell_floats::CellFloatJob {
                    layout: set.layout,
                    frame,
                    page: set.page,
                    paragraph: &paragraph,
                    resolved: &resolved,
                    line: anchored,
                };
                cell_floats::cell_floats(&job, to.out);
            }
        }
        for slot in to.out.drawings.get_mut(from..).unwrap_or_default() {
            slot.story = set.story.clone();
        }
        if let LineSource::Flow { item } = line.source {
            let next_same = set
                .lines
                .get(k + 1)
                .is_some_and(|n| n.source == LineSource::Flow { item });
            if !next_same {
                paragraph_rules(set, model, document, (k, item, width(line.column)), to.out);
            }
        }
    }
    table_rules(set, to.out);
    if let Some((_, part)) = &set.story {
        // A header's or footer's picture is related from the story's own part.
        for picture in to.out.pictures.get_mut(pictures_from..).unwrap_or_default() {
            picture.part = Some(part.clone());
        }
    }
}

/// The numbers of page `page`'s lines put before their spans (`_line_numbers`), the text
/// column starting at `left`.
fn number_lines(
    layout: &DocumentLayout,
    model: &mut Model,
    (page, left): (usize, Px),
    cx: &PaintContext<'_>,
    lines: &mut [PaintedLine],
) {
    let numbers = layout.line_numbers(model);
    for number in numbers.on(page) {
        let line = lines.iter_mut().find(|l| {
            l.story.is_none() && l.name == number.name && l.number == number.line && l.baseline == number.baseline
        });
        if let (Some(line), Some(span)) = (line, line_numbers::number_span(cx, layout, model, number, left)) {
            line.spans.insert(0, span);
        }
    }
}

/// The body's lines of the page `geometry` is of, to paint in `frame` (its frame).
fn body_set<'a>(
    layout: &'a DocumentLayout,
    geometry: &'a PageGeometry,
    frame: &'a crate::geometry::PageFrame,
) -> LineSet<'a> {
    let laid = layout.laid_page(geometry.page);
    LineSet {
        items: &layout.flow.items,
        blocks: &layout.document.body,
        paragraphs: std::borrow::Cow::Borrowed(&**layout.body_paragraphs()),
        laid,
        broken: laid.map(|l| &l.broken),
        lines: &geometry.lines,
        rows: laid
            .map(|l| l.rows.iter().map(|r| (r.item, &r.row, r.room.as_ref())).collect())
            .unwrap_or_default(),
        story: None,
        frame: Some(frame),
        clip: None,
        page: geometry.page,
        paragraph_tops: &[],
        layout,
    }
}

/// The lines of a header, a footer, a page's footnotes or its opening line, to paint, on
/// page `page` of frame `frame` (where a header's or footer's floating drawings are
/// placed).
fn story_set<'a>(
    layout: &'a DocumentLayout,
    story: &'a StoryPage,
    (page, frame): (usize, &'a crate::geometry::PageFrame),
) -> LineSet<'a> {
    let header_or_footer = matches!(story.kind, "header" | "footer");
    LineSet {
        items: &story.flow.items,
        blocks: &story.blocks,
        paragraphs: body_paragraphs(&story.blocks).0.into(),
        laid: None,
        broken: Some(&story.broken),
        lines: &story.walked.lines,
        rows: story.walked.rows.iter().map(|(i, r)| (*i, r, None)).collect(),
        story: header_or_footer.then(|| (story.kind, story.part.clone())),
        frame: header_or_footer.then_some(frame),
        clip: story.clip,
        page,
        paragraph_tops: &story.walked.paragraph_tops,
        layout,
    }
}

/// Page `page`'s paint (0-based, blank pages counted): its body's lines, then its header's
/// and footer's, then its footnotes (`parts`, from [`crate::WordLayout::page_parts`]),
/// their paragraphs resolved in `model`.
pub fn paint_page(
    layout: &DocumentLayout,
    model: &mut Model,
    page: usize,
    parts: &PageParts,
    cx: &PaintContext<'_>,
) -> Option<PagePaint> {
    let geometry = layout.page_geometry(page)?;
    let document: &WordDocument = &layout.document;
    let laid = layout.laid_page(page);
    let mut out = spans::PageOut::default();
    let mut lines = Vec::new();
    let frame = geometry.frame;
    let width = |column: Option<usize>| match column {
        Some(c) => layout
            .sections
            .get(frame.section)
            .map_or(Px::ZERO, |s| Px::twips(s.column_width(c))),
        None => frame.right - frame.left,
    };
    if let Some(opening) = &parts.opening {
        paint_set(
            &story_set(layout, opening, (page, &frame)),
            model,
            document,
            (cx, &width),
            PaintOut {
                out: &mut out,
                lines: &mut lines,
            },
        );
    }
    let body = body_set(layout, &geometry, &frame);
    paint_set(
        &body,
        model,
        document,
        (cx, &width),
        PaintOut {
            out: &mut out,
            lines: &mut lines,
        },
    );
    if let Some(laid) = laid {
        out.rules.extend(separators::column_separators(layout, laid));
        // docx2svg draws a marked box of a placeholder's extent, which Word does not: not
        // painted here.
        for (_, placeholder) in crate::placeholder::placeholders_on(&layout.flow.items, laid) {
            out.unpainted
                .push(format!("a placeholder of {}", placeholder.path(&layout.document.names)));
        }
    }
    number_lines(layout, model, (page, frame.left), cx, &mut lines);
    out.unpainted.extend(parts.unpainted.iter().cloned());
    for story in parts.stories.iter().chain(&parts.footnotes) {
        paint_set(
            &story_set(layout, story, (page, &frame)),
            model,
            document,
            (cx, &width),
            PaintOut {
                out: &mut out,
                lines: &mut lines,
            },
        );
    }
    Some(PagePaint {
        geometry,
        lines,
        rules: out.rules,
        pictures: out.pictures,
        drawings: out.drawings,
        unpainted: out.unpainted,
        unplaced: out.unplaced,
        text_boxes: Vec::new(),
    })
}
