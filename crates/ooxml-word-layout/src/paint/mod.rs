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

mod decorate;
mod paragraphs;
mod positions;
mod spans;
pub mod svg;
mod tables;

pub use positions::{line_positions, Positions, Setting, TabSpan};

use crate::engine::DocumentLayout;
use crate::faces::Faces;
use crate::flow::{body_paragraphs, FlowItem};
use crate::geometry::{DrawnLine, LineSource, PageGeometry};
use crate::px::Px;
use ooxml_word_model::model::Paragraph;
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
        }
    }
}

/// Consecutive glyphs of one run, in one format, on one line (`layout.Span`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// The run's path (the paragraph's for a label).
    pub path: String,
    /// Its characters.
    pub chars: Vec<char>,
    /// Each glyph's pen x, px from the page's left edge.
    pub xs: Vec<Px>,
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
}

/// A filled rectangle the text needs (`layout.Rule`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// What it is: `underline`, `strike`, `highlight`, `shading`, `paragraph-border`,
    /// `paragraph-shading`, `table-border`, `cell-shading`, `separator`.
    pub kind: &'static str,
    /// The path of what it is drawn for.
    pub path: String,
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
}

/// A drawing a page draws, by its extent: what a painter of pictures, DrawingML, charts
/// or SmartArt fills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawn {
    /// Its path (`.../w:r[2]` of an inline one).
    pub path: String,
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

/// One line drawn, with its spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaintedLine {
    /// Its paragraph's path.
    pub path: Text,
    /// Its paragraph's `w14:paraId`.
    pub paragraph_id: Option<Text>,
    /// Its number in the paragraph.
    pub number: usize,
    /// Its baseline.
    pub baseline: i64,
    /// Its spans.
    pub spans: Vec<Span>,
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
    /// Its inline pictures and drawings.
    pub pictures: Vec<Drawn>,
    /// What the page holds that this paint does not draw yet, by reason: the page's SVG is
    /// not complete where any is listed.
    pub unpainted: Vec<String>,
}

/// How the SVG names a face a rasteriser does not find by the document's name
/// (docx2svg's `fonts.DrawingName`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingName {
    /// The family a rasteriser files the face under.
    pub family: String,
    /// Its CSS weight.
    pub weight: u16,
    /// Its CSS stretch.
    pub stretch: String,
    /// Its CSS style.
    pub style: String,
}

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

/// The paragraph a drawn line is of, and its measure on the page.
fn paragraph_of(
    layout: &DocumentLayout,
    page: &PageGeometry,
    line: &DrawnLine,
    paragraphs: &[Arc<Paragraph>],
) -> Option<(Arc<Paragraph>, Arc<crate::paragraph::MeasuredParagraph>)> {
    match line.source {
        LineSource::Flow { item } => {
            let laid = layout.laid_page(page.page)?;
            let measured = crate::geometry::measured_on(layout, laid, item)?;
            Some((Arc::clone(paragraphs.get(line.block?)?), measured))
        }
        LineSource::Cell { item, block } => {
            let Some(FlowItem::Table(t)) = layout.flow.items.get(item) else {
                return None;
            };
            Some((Arc::clone(paragraphs.get(line.block?)?), t.flow.paragraph(block)?))
        }
        LineSource::DropCap { item } => {
            let drop = layout.flow.items.get(item)?.para()?.drop_cap.clone()?;
            match layout.document.body.get(drop.body) {
                Some(ooxml_word_model::model::Block::Paragraph(p)) => Some((Arc::clone(p), Arc::clone(&drop.measured))),
                _ => None,
            }
        }
        LineSource::Opening { .. } => None,
    }
}

/// A body paragraph's borders and shading on the page, after its last line there
/// (`_paragraph_rules`); line `last` of the page is its last there.
fn paragraph_rules(
    layout: &DocumentLayout,
    model: &mut Model,
    geometry: &PageGeometry,
    (last, item): (usize, usize),
    out: &mut spans::PageOut,
) {
    let Some(p) = layout.flow.items.get(item).and_then(FlowItem::para) else {
        return;
    };
    let document = &layout.document;
    let mine: Vec<&DrawnLine> = geometry
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
    let mut resolve = |k: Option<usize>| match k.and_then(|k| document.body.get(k)) {
        Some(ooxml_word_model::model::Block::Paragraph(q)) => Some(model.resolve_paragraph(document, q).paragraph),
        _ => None,
    };
    let Some(pp) = resolve(Some(p.body)) else { return };
    let previous = resolve(p.body.checked_sub(1));
    let following = resolve(Some(p.body + 1));
    let (before_same, after_same) = crate::borders::box_neighbours(&pp, previous.as_deref(), following.as_deref());
    let starts_here = first.number == 0;
    let ends_here = end.number + 1 == p.count();
    let width = match first.column {
        Some(c) => layout
            .sections
            .get(geometry.frame.section)
            .map_or(Px::ZERO, |s| Px::twips(s.column_width(c))),
        None => geometry.frame.right - geometry.frame.left,
    };
    let (left, right) = paragraphs::text_edges(&pp, (first.left, first.left + width));
    let bx = paragraphs::ParagraphBox {
        pp: &pp,
        path: &paths_of(document, p.block),
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

/// The page's tables' borders and shading, row by row (a row's nested tables' first).
fn table_rules(layout: &DocumentLayout, laid: &crate::paginate::LaidPage, out: &mut spans::PageOut) {
    let mut rules = tables::TableRules::default();
    let mut k = 0;
    while let Some(first) = laid.rows.get(k) {
        let item = first.item;
        let run: Vec<&crate::table::PlacedRow> = laid
            .rows
            .get(k..)
            .unwrap_or_default()
            .iter()
            .take_while(|r| r.item == item)
            .map(|r| &r.row)
            .collect();
        k += run.len();
        let Some(FlowItem::Table(t)) = layout.flow.items.get(item) else {
            continue;
        };
        let Some(table) = t.flow.as_any().downcast_ref::<crate::tables::MeasuredTable>() else {
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
fn rows_rules(table: &crate::tables::MeasuredTable, rows: &[&crate::table::PlacedRow], out: &mut tables::TableRules) {
    for (k, piece) in rows.iter().enumerate() {
        for (nested, top) in tables::nested_tables(table, piece) {
            let placed = tables::whole_rows(nested, top);
            let refs: Vec<&crate::table::PlacedRow> = placed.iter().collect();
            rows_rules(nested, &refs, out);
        }
        let bottom = piece.top + piece.top_border + piece.height;
        let parted = rows.get(k + 1).is_some_and(|next| next.top != bottom);
        let last = k + 1 == rows.len() || parted || piece.row + 1 == table.rows.len();
        tables::row_rules(table, piece, bottom, last, out);
    }
}

/// The path of the body's paragraph `block` (cells included).
fn paths_of(document: &WordDocument, block: usize) -> String {
    body_paragraphs(&document.body)
        .0
        .get(block)
        .map(|p| p.path.to_string())
        .unwrap_or_default()
}

/// Page `page`'s paint (0-based, blank pages counted), its paragraphs resolved in `model`.
pub fn paint_page(layout: &DocumentLayout, model: &mut Model, page: usize, cx: &PaintContext<'_>) -> Option<PagePaint> {
    let geometry = layout.page_geometry(page)?;
    let document: &WordDocument = &layout.document;
    let (paragraphs, _) = body_paragraphs(&document.body);
    let mut out = spans::PageOut::default();
    let mut lines = Vec::new();
    for (k, line) in geometry.lines.iter().enumerate() {
        let Some((paragraph, measured)) = paragraph_of(layout, &geometry, line, &paragraphs) else {
            out.unpainted.push(format!("{:?}", line.source));
            continue;
        };
        let resolved = model.resolve_paragraph(document, &paragraph);
        let job = spans::LineJob {
            document,
            paragraph: &paragraph,
            resolved: &resolved,
            measured: &measured,
            line,
        };
        if let Some(painted) = spans::paint_line(cx, &job, &mut out) {
            lines.push(painted);
        }
        if let LineSource::Flow { item } = line.source {
            let next_same = geometry
                .lines
                .get(k + 1)
                .is_some_and(|n| n.source == LineSource::Flow { item });
            if !next_same {
                paragraph_rules(layout, model, &geometry, (k, item), &mut out);
            }
        }
    }
    if let Some(laid) = layout.laid_page(page) {
        table_rules(layout, laid, &mut out);
    }
    Some(PagePaint {
        geometry,
        lines,
        rules: out.rules,
        pictures: out.pictures,
        unpainted: out.unpainted,
    })
}
