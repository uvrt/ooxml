//! Owns: the floating drawings anchored on a table cell's line, placed on the page as
//! docx2svg's `_cell_anchors` places them (`make_cell_anchor_probe.py`, ROADMAP F.16):
//! in the cell (`layoutInCell`, and in mode 15 whatever it says) `column` and `margin` are
//! its text area, `page` and `leftMargin` start inside its left border, `paragraph` is the
//! cell paragraph's, `line` the line's, and `margin`, `page` and `topMargin` start below
//! the row's top border -- in a cell aligned vertically as much lower as its lines; below
//! mode 15 without `layoutInCell` every frame is the page's, but `paragraph` is the row's
//! top. A drawing text wraps around in the cell is drawn where the cell's layout put it
//! (`wrap_cell`).
//!
//! Must not: lay the cell out (`crate::tables` has), or paint a drawing.

use super::drawings::{anchor_box, anchor_path, character_px, page_frames, DrawingSlot, Layer, SlotName};
use super::spans::PageOut;
use crate::geometry::PageFrame;
use crate::tables::{grid_twips_px, in_cell, AnchoredLine, CellPage};
use ooxml_base::Q;
use ooxml_word_model::model::Paragraph;
use ooxml_word_model::value::Value;
use ooxml_word_model::ResolvedParagraph;

/// What placing a cell line's drawings reads.
pub(super) struct CellFloatJob<'a> {
    pub(super) layout: &'a crate::DocumentLayout,
    pub(super) frame: &'a PageFrame,
    pub(super) page: usize,
    pub(super) paragraph: &'a Paragraph,
    pub(super) resolved: &'a ResolvedParagraph,
    pub(super) line: &'a AnchoredLine<'a>,
}

/// The floating drawings anchored on a cell line, placed (`_cell_anchors`); a drawing
/// positioned against a frame the model does not know is named in `out.unplaced`.
pub(super) fn cell_floats(job: &CellFloatJob<'_>, out: &mut PageOut) {
    let line = job.line;
    let Some(props) = job.layout.sections.get(job.frame.section).map(|s| &s.props) else {
        return;
    };
    let mode15 = job.layout.mode15;
    let px = Q::new(5, 24);
    let page = CellPage {
        width: props.page_size.width,
        height: props.page_size.height,
        header: props.margins.header,
        footer: props.margins.footer,
    };
    let alignment = job.resolved.paragraph.get("jc").and_then(Value::str);
    let line_top = if line.line == 0 { line.paragraph_top } else { line.top };
    let extra = line.entry.drawings.extra.get(line.line);
    for a in line.entry.drawings.anchors.iter().filter(|a| a.line == line.line) {
        let Some(character) = character_px(
            line.measured,
            (line.line, extra, alignment),
            (a.source, a.run),
            grid_twips_px(line.edges.text_left),
        ) else {
            continue;
        };
        let frames = if in_cell(&a.anchor, mode15) {
            let moved = (line.cell_top + line.offset).value() / px;
            Some(line.edges.frames(
                &page,
                (moved, job.page + 1, mode15),
                [
                    line.paragraph_top.value() / px,
                    line_top.value() / px,
                    (line.top + line.pitch).value() / px,
                    character.value() / px,
                ],
            ))
        } else {
            let tops = [line.row_top, line_top, line.top + line.pitch];
            page_frames(job.layout, (job.frame.section, job.page), tops, character)
        };
        let Some((x, y, width, height)) = frames.and_then(|f| anchor_box(&a.anchor, &f)) else {
            out.unplaced
                .push(anchor_path(&job.layout.document.names, job.paragraph, a.run, a.index));
            continue;
        };
        let positioned = line
            .entry
            .drawings
            .positioned
            .iter()
            .find(|p| p.at == (a.source, a.run, a.index));
        let (x, y) = match positioned {
            // One text wraps around: where the cell's layout without it put it.
            Some(p) => (p.x, line.cell_top + line.offset + p.y),
            None => (x, y),
        };
        out.drawn += 1;
        out.drawings.push(DrawingSlot {
            name: SlotName::of(job.paragraph, a.run, a.index, false),
            run: a.run,
            index: a.index,
            inline: false,
            x,
            y,
            width,
            height,
            layer: if a.anchor.behind { Layer::Behind } else { Layer::Front },
            order: (a.anchor.relative_height, out.drawn),
            story: None,
            graphic: a.anchor.graphic.clone(),
            no_overlap: !a.anchor.allow_overlap,
        });
    }
}
