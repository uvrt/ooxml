//! Owns: where a page's drawings go -- the seam with the painters of DrawingML, charts and
//! SmartArt ([`DrawingSlot`]): an inline shape, group, chart or diagram in its line's box,
//! and a floating drawing (`wp:anchor`) positioned on its page as docx2svg's `_anchors` and
//! `_place_float` position it (`floating`'s frames against the page, its paragraph, its
//! line and its character; a drawing text wraps top and bottom of where the paginator put
//! it), each with its layer and paint order.
//!
//! Must not: paint a drawing (its painter does, from the slot), or read its parts.

use super::positions::{line_positions, Setting};
use super::spans::picture_size;
use crate::floating::{emu_twips, horizontal, vertical, AnchorRef, Frames};
use crate::geometry::{DrawnLine, PageFrame};
use crate::paragraph::MeasuredParagraph;
use crate::px::Px;
use ooxml_base::Q;
use ooxml_text::{Geometry, Piece, Units};
use ooxml_word_model::model::{Anchor, Graphic, Paragraph};
use ooxml_word_model::value::{Text, Value};
use ooxml_word_model::ResolvedParagraph;
use std::sync::Arc;

/// Which layer a drawing paints in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// Behind the text (`behindDoc`).
    Behind,
    /// In front of it (an inline drawing too).
    Front,
}

/// A drawing a page draws, by where it goes: what its painter fills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingSlot {
    /// Its path (`.../w:r[2]/wp:anchor[1]`, or an inline one's run's).
    pub path: String,
    /// Its paragraph's path.
    pub paragraph_path: String,
    /// Its run's index in the paragraph.
    pub run: usize,
    /// Its index among the run's anchors (or inline drawings).
    pub index: usize,
    /// Inline in its line, not floating.
    pub inline: bool,
    /// Its box's left edge, px from the page's left.
    pub x: Px,
    /// Its top.
    pub y: Px,
    /// Its width.
    pub width: Px,
    /// Its height.
    pub height: Px,
    /// Its layer.
    pub layer: Layer,
    /// `(relativeHeight, the order it was met in)`: how drawings of a layer stack.
    pub order: (i64, usize),
    /// A header's or footer's drawing: the story's kind and part.
    pub story: Option<(&'static str, Text)>,
    /// What it shows, as the model read it (`None`: an anchor whose graphic was not read).
    pub graphic: Option<Arc<Graphic>>,
    /// Whether text may not overlap it (`allowOverlap="0"`): Word moves it clear.
    pub no_overlap: bool,
}

/// The pen x, layout units from the column's left, of the character a drawing anchored
/// at `source` of run `run` is positioned against (`anchor_character_units`).
fn character_units(
    pieces: &[Piece],
    line: &ooxml_text::Line,
    positions: &std::collections::BTreeMap<usize, Units>,
    (source, run): (i64, usize),
    (geometry, first_line): (&Geometry, bool),
) -> Units {
    let run = i64::try_from(run).unwrap_or(i64::MAX);
    let before = (line.start..line.end).rfind(|j| {
        positions.contains_key(j)
            && pieces
                .get(*j)
                .is_some_and(|p| p.source >= 0 && (p.source, p.run) < (source, run))
    });
    if let Some(j) = before {
        return positions.get(&j).copied().unwrap_or(Units::ZERO);
    }
    if let Some(x) = (line.start..line.end).find_map(|j| positions.get(&j).copied()) {
        return x;
    }
    if first_line {
        geometry.first_start
    } else {
        geometry.start
    }
}

/// The pen x, px from the page's left, of the character a drawing anchored at `source` of
/// run `run` on line `number` of a measured paragraph is positioned against, its column's
/// text starting at `left` (`anchor_character_units` over `wrapped_positions`): what a
/// drawing anchored in a table cell is positioned against.
pub(crate) fn character_px(
    measured: &MeasuredParagraph,
    (number, extra, alignment): (usize, Option<&crate::rebreak::LineExtra>, Option<&str>),
    (source, run): (i64, usize),
    left: Px,
) -> Option<Px> {
    let line = measured.measured.lines.get(number)?;
    let setting = Setting {
        alignment,
        first_line: number == 0,
        last_line: number + 1 == measured.measured.lines.len(),
        decimal: '.',
    };
    let pieces = &measured.measured.pieces;
    let geometry = &measured.geometry;
    let positions = super::positions::wrapped_positions(pieces, line, geometry, extra, &setting).at;
    let units = character_units(pieces, line, &positions, (source, run), (geometry, number == 0));
    Some(super::spans::px_at(left, units))
}

/// A floating drawing's path (`anchor_path`): its `wp:anchor[n]` among the run's DrawingML
/// ones, or a VML shape's own path.
pub fn anchor_path(paragraph: &Paragraph, run: usize, k: usize) -> String {
    let Some(r) = paragraph.runs.get(run).filter(|r| !r.path.is_empty()) else {
        return paragraph.path.to_string();
    };
    let vml = r
        .anchors
        .get(k)
        .filter(|a| a.vml)
        .and_then(|a| a.graphic.as_deref())
        .filter(|g| !g.path.is_empty());
    if let Some(graphic) = vml {
        return format!("{}/{}", paragraph.path, graphic.path);
    }
    let n = r.anchors.iter().take(k).filter(|a| !a.vml).count();
    format!("{}/{}/wp:anchor[{}]", paragraph.path, r.path, n + 1)
}

/// The drawing's box, px: where `floating` puts it, at the size Word draws it.
pub(crate) fn anchor_box(anchor: &Anchor, frames: &Frames) -> Option<(Px, Px, Px, Px)> {
    let twip = |q: Q| Px::from(q * Q::new(5, 24));
    let (x, y) = (horizontal(anchor, frames)?, vertical(anchor, frames)?);
    let picture = anchor.graphic.as_ref().is_some_and(|g| &*g.kind == "picture");
    let (width, height) = if picture {
        picture_size(anchor.extent.0, anchor.extent.1)
    } else {
        let t = |v: i64| twip(Q::from_integer(i128::from(emu_twips(v))));
        (t(anchor.extent.0), t(anchor.extent.1))
    };
    Some((twip(x), twip(y), width, height))
}

/// What placing a paragraph line's floating drawings reads.
pub(super) struct FloatJob<'a> {
    pub(super) layout: &'a crate::DocumentLayout,
    pub(super) frame: &'a PageFrame,
    /// The page (0-based).
    pub(super) page: usize,
    /// The paragraph's top for its drawings, where the page placed it.
    pub(super) paragraph_top: Option<Px>,
    /// The bands the paginator put drawings text wraps top and bottom of in.
    pub(super) bands: &'a [crate::paginate::Band],
    /// The drawings the paginator put text beside.
    pub(super) wraps: &'a [crate::wrap::Wrap],
    pub(super) paragraph: &'a Paragraph,
    pub(super) resolved: &'a ResolvedParagraph,
    pub(super) measured: &'a MeasuredParagraph,
    pub(super) anchors: &'a [AnchorRef],
    pub(super) item: usize,
    pub(super) line: &'a DrawnLine,
}

/// The floating drawings anchored on a line of the body or of a header or footer, placed
/// (`_anchors`); `count` the
/// page's drawings so far. A drawing positioned against a frame the model does not know
/// is not placed, and named in the second list.
pub(super) fn line_floats(job: &FloatJob<'_>, count: &mut usize) -> (Vec<DrawingSlot>, Vec<String>) {
    let (mut out, mut missed) = (Vec::new(), Vec::new());
    let number = job.line.number;
    let mine: Vec<&AnchorRef> = job.anchors.iter().filter(|a| a.line == number).collect();
    let (Some(section), Some(broken)) = (
        job.layout.sections.get(job.frame.section),
        job.measured.measured.lines.get(number),
    ) else {
        return (out, missed);
    };
    if mine.is_empty() {
        return (out, missed);
    }
    let paragraph_top = job.paragraph_top.unwrap_or(job.line.top);
    let setting = Setting {
        alignment: job.resolved.paragraph.get("jc").and_then(Value::str),
        first_line: number == 0,
        last_line: number + 1 == job.measured.measured.lines.len(),
        decimal: '.',
    };
    let geometry = &job.measured.geometry;
    let positions = line_positions(&job.measured.measured.pieces, broken, geometry, &setting).at;
    let m = &section.props.margins;
    let exact_left = Px::twips(m.left + m.gutter);
    let line_top = if number == 0 { paragraph_top } else { job.line.top };
    for a in mine {
        let units = character_units(
            &job.measured.measured.pieces,
            broken,
            &positions,
            (a.source, a.run),
            (geometry, number == 0),
        );
        let character = super::spans::px_at(exact_left, units);
        let Some(frames) = job.frames((paragraph_top, line_top), character) else {
            continue;
        };
        let path = anchor_path(job.paragraph, a.run, a.index);
        let Some((x, y, width, height)) = anchor_box(&a.anchor, &frames) else {
            missed.push(path);
            continue;
        };
        let key = crate::paginate::DrawingKey {
            block: job
                .layout
                .flow
                .items
                .get(job.item)
                .map_or(0, crate::flow::FlowItem::block),
            source: a.source,
            run: a.run,
            index: a.index,
        };
        let (x, y) = match job.placed(key) {
            Some((left, top)) => (left.unwrap_or(x), top),
            None => (x, y),
        };
        *count += 1;
        out.push(DrawingSlot {
            path,
            paragraph_path: job.paragraph.path.to_string(),
            run: a.run,
            index: a.index,
            inline: false,
            x,
            y,
            width,
            height,
            layer: if a.anchor.behind { Layer::Behind } else { Layer::Front },
            order: (a.anchor.relative_height, *count),
            story: None,
            graphic: a.anchor.graphic.clone(),
            no_overlap: !a.anchor.allow_overlap,
        });
    }
    (out, missed)
}

impl FloatJob<'_> {
    /// Where the paginator positioned drawing `key`, where it did: the top of one text
    /// wraps top and bottom of, the left edge and top of one text wraps beside.
    fn placed(&self, key: crate::paginate::DrawingKey) -> Option<(Option<Px>, Px)> {
        if let Some(band) = self.bands.iter().find(|b| b.key == key) {
            return Some((None, band.y));
        }
        self.wraps.iter().find_map(|w| {
            w.drawing
                .filter(|(k, _)| *k == key)
                .map(|(_, x)| (Some(Px::from(x)), Px::from(w.y)))
        })
    }

    /// What an anchor on this page is positioned against (`_frames`): the paragraph's
    /// and line's tops and the character's x given in px, exact.
    fn frames(&self, (paragraph_top, line_top): (Px, Px), character: Px) -> Option<Frames> {
        let tops = [paragraph_top, line_top, self.line.top + self.line.pitch];
        page_frames(self.layout, (self.frame.section, self.page), tops, character)
    }
}

/// What an anchor on page `page` (0-based) of section `section` is positioned against
/// (`_frames`): the page's margins, the paragraph's and line's tops and the line's foot
/// (`tops`) and the character's x given in px, exact.
pub(super) fn page_frames(
    layout: &crate::DocumentLayout,
    (section, page): (usize, usize),
    tops: [Px; 3],
    character: Px,
) -> Option<Frames> {
    let props = &layout.sections.get(section)?.props;
    let m = &props.margins;
    let to_twips = |v: Px| v.value() / Q::new(5, 24);
    let [paragraph_top, line_top, line_bottom] = tops;
    Some(Frames {
        page_width: props.page_size.width,
        page_height: props.page_size.height,
        left: Q::from_integer(i128::from(m.left + m.gutter)),
        right: Q::from_integer(i128::from(m.right)),
        top: Q::from_integer(i128::from(m.top)),
        bottom: Q::from_integer(i128::from(m.bottom)),
        header: m.header,
        footer: m.footer,
        page: page + 1,
        mode15: layout.mode15,
        paragraph_top: to_twips(paragraph_top),
        line_top: to_twips(line_top),
        line_bottom: to_twips(line_bottom),
        character: to_twips(character),
        column: None,
        origin: (Q::from_integer(0), Q::from_integer(0)),
        cell: false,
    })
}
