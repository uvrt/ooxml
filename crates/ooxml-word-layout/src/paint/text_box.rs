//! Owns: a text box's text painted where its story was laid out (docx2svg's
//! `_Placer._text_box` walking its scratch page): its lines' spans, its paragraphs' and
//! tables' rules, its pictures and its inline drawings that are not pictures (a chart, a
//! shape: for their painters, drawn with the box as the scratch page's floats are), as a
//! page's are painted; its automatic text in the colour its shape gives it; what of it is
//! not drawn.
//!
//! Must not: lay the text out (`crate::geometry::text_box` has), or paint the shape.

use super::drawings::DrawingSlot;
use super::{body_paragraphs, paint_set, LineSet, PaintContext, PaintOut, PaintedLine, Rule};
use crate::engine::DocumentLayout;
use crate::geometry::text_box::TextBoxStory;
use crate::paint::Drawn;
use ooxml_word_model::value::Text;
use ooxml_word_model::Model;

/// A text box's paint, for the drawing it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBoxPaint {
    /// The drawing's index among the page's drawings.
    pub slot: usize,
    /// Its lines.
    pub lines: Vec<PaintedLine>,
    /// Its rules.
    pub rules: Vec<Rule>,
    /// Its inline pictures.
    pub pictures: Vec<Drawn>,
    /// Its inline drawings that are not pictures, in the order met, for their painters:
    /// drawn in the box's drawing after its pictures (docx2svg's scratch page's floats,
    /// whose primitives go with the text box), their story the box's drawing's.
    pub drawings: Vec<DrawingSlot>,
    /// What is not drawn as docx2svg draws it.
    pub unpainted: Vec<String>,
}

/// Paint text box `story` of drawing `slot` on page `page`; `drawn_in`: the story a
/// header's or footer's drawing is in, its kind and part (its lines are that story's, its
/// pictures related from its part).
pub fn paint_text_box(
    layout: &DocumentLayout,
    model: &mut Model,
    (slot, page, drawn_in): (usize, usize, Option<&(&'static str, Text)>),
    story: &TextBoxStory,
    cx: &PaintContext<'_>,
) -> TextBoxPaint {
    let mut out = TextBoxPaint {
        slot,
        lines: Vec::new(),
        rules: Vec::new(),
        pictures: Vec::new(),
        drawings: Vec::new(),
        unpainted: Vec::new(),
    };
    let Some(laid) = &story.story else {
        return out;
    };
    if crate::geometry::text_box::stopped(laid) {
        out.unpainted.push("a text box whose text stops".into());
    }
    let set = LineSet {
        items: &laid.flow.items,
        blocks: &laid.blocks,
        paragraphs: body_paragraphs(&laid.blocks).0.into(),
        laid: None,
        broken: Some(&laid.broken),
        lines: &laid.walked.lines,
        rows: laid.walked.rows.iter().map(|(i, r)| (*i, r, None)).collect(),
        story: drawn_in.cloned(),
        frame: None,
        clip: None,
        page,
        paragraph_tops: &laid.walked.paragraph_tops,
        layout,
    };
    let mut page_out = super::spans::PageOut::default();
    let width = story.width;
    paint_set(
        &set,
        model,
        &layout.document,
        (cx, &|_| width),
        PaintOut {
            out: &mut page_out,
            lines: &mut out.lines,
        },
    );
    if let Some(colour) = &story.colour {
        for span in out.lines.iter_mut().flat_map(|l| l.spans.iter_mut()) {
            if span.color.is_none() {
                span.color = Some(colour.clone());
            }
        }
    }
    // A floating drawing in the box is not drawn (its mark was taken out as it was laid
    // out, and said so); its inline ones are.
    out.drawings = page_out
        .drawings
        .into_iter()
        .filter(|slot| slot.inline)
        .map(|slot| DrawingSlot {
            story: drawn_in.cloned(),
            ..slot
        })
        .collect();
    out.unpainted.extend(page_out.unpainted);
    out.rules = page_out.rules;
    out.pictures = page_out.pictures;
    out
}
