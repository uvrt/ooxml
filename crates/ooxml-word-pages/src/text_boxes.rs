//! Owns: a page's text boxes laid out and painted ([`text_boxes`]): each drawing's text
//! frames, laid out by the version's layout engine and painted with the page's context.
//!
//! Must not: keep what it paints (the engine's paint memo does), or say warnings (the
//! warnings' drawings port does).

use crate::drawings::WordDrawings;
use ooxml_word_layout::paint::text_box::{paint_text_box, TextBoxPaint};
use ooxml_word_layout::paint::{PagePaint, PaintContext};
use ooxml_word_layout::WordLayout;
use ooxml_word_model::Model;

/// What laying a page's text boxes out needs: the layout engine, the model, the version
/// laid out, and the paint's context.
pub(crate) struct TextBoxJob<'a, 'c> {
    pub(crate) layout: &'a mut WordLayout,
    pub(crate) model: &'a mut Model,
    pub(crate) laid: &'a ooxml_word_layout::DocumentLayout,
    pub(crate) cx: &'a PaintContext<'c>,
}

/// The text boxes of a page's drawings, laid out and painted.
pub(crate) fn text_boxes(
    job: TextBoxJob<'_, '_>,
    drawings: &WordDrawings,
    paint: &PagePaint,
    page: usize,
) -> Vec<TextBoxPaint> {
    let mut out = Vec::new();
    for (k, slot) in paint.drawings.iter().enumerate() {
        for frame in drawings.text_frames(slot) {
            let story = job
                .layout
                .text_box(job.model, job.laid, (&slot.path, &slot.paragraph_path), &frame);
            out.push(paint_text_box(
                job.laid,
                job.model,
                (k, page, slot.story.as_ref()),
                &story,
                job.cx,
            ));
        }
    }
    out
}
