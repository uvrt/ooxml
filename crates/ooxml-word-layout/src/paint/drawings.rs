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
use crate::paginate::LaidPage;
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

/// A floating drawing's path (`anchor_path`): its `wp:anchor[n]` among the run's DrawingML
/// ones.
fn anchor_path(paragraph: &Paragraph, run: usize, k: usize) -> String {
    let Some(r) = paragraph.runs.get(run).filter(|r| !r.path.is_empty()) else {
        return paragraph.path.to_string();
    };
    let n = r.anchors.iter().take(k).filter(|a| !a.vml).count();
    format!("{}/{}/wp:anchor[{}]", paragraph.path, r.path, n + 1)
}

/// The drawing's box, px: where `floating` puts it, at the size Word draws it.
fn anchor_box(anchor: &Anchor, frames: &Frames) -> Option<(Px, Px, Px, Px)> {
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
    pub(super) laid: &'a LaidPage,
    pub(super) paragraph: &'a Paragraph,
    pub(super) resolved: &'a ResolvedParagraph,
    pub(super) measured: &'a MeasuredParagraph,
    pub(super) anchors: &'a [AnchorRef],
    pub(super) item: usize,
    pub(super) line: &'a DrawnLine,
}

/// The floating drawings anchored on a body line, placed (`_anchors`); `count` the
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
    let placed = job.laid.lines.iter().find(|l| l.item == job.item && l.line == number);
    let paragraph_top = placed.map_or(job.line.top, |p| p.paragraph_top);
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
        let Some((x, mut y, width, height)) = anchor_box(&a.anchor, &frames) else {
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
        if let Some(band) = job.laid.bands.iter().find(|b| b.key == key) {
            y = band.y;
        }
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
    /// What an anchor on this page is positioned against (`_frames`): the paragraph's
    /// and line's tops and the character's x given in px, exact.
    fn frames(&self, (paragraph_top, line_top): (Px, Px), character: Px) -> Option<Frames> {
        let props = &self.layout.sections.get(self.frame.section)?.props;
        let m = &props.margins;
        let to_twips = |v: Px| v.value() / Q::new(5, 24);
        Some(Frames {
            page_width: props.page_size.width,
            page_height: props.page_size.height,
            left: m.left + m.gutter,
            right: m.right,
            top: Q::from_integer(i128::from(m.top)),
            bottom: Q::from_integer(i128::from(m.bottom)),
            header: m.header,
            footer: m.footer,
            page: self.laid.page + 1,
            mode15: self.layout.mode15,
            paragraph_top: to_twips(paragraph_top),
            line_top: to_twips(line_top),
            line_bottom: to_twips(self.line.top + self.line.pitch),
            character: to_twips(character),
            column: None,
        })
    }
}
