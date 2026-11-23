//! Owns: a page's stories painted on their own ([`paint_stories`]): its header's and
//! footer's lines and its footnotes, as [`super::paint_page`] paints them after the body
//! (the same sets, in the same order). A reader of a page whose body is laid out as before
//! but whose stories show other values (another page count in "Page x of N") paints only
//! these again.
//!
//! Must not: paint the body, its opening or its line numbers.

use super::{paint_set, story_set, PagePaint, PaintContext, PaintOut};
use crate::engine::DocumentLayout;
use crate::geometry::stories::PageParts;
use crate::px::Px;
use ooxml_word_model::Model;

/// Page `page`'s stories and footnotes (`parts`) painted as [`super::paint_page`] paints
/// them after its body: its paint holds only their lines, rules, pictures and drawings.
pub fn paint_stories(
    layout: &DocumentLayout,
    model: &mut Model,
    page: usize,
    parts: &PageParts,
    cx: &PaintContext<'_>,
) -> Option<PagePaint> {
    let geometry = layout.page_geometry(page)?;
    let frame = geometry.frame;
    let width = |column: Option<usize>| match column {
        Some(c) => layout
            .sections
            .get(frame.section)
            .map_or(Px::ZERO, |s| Px::twips(s.column_width(c))),
        None => frame.right - frame.left,
    };
    let mut out = super::spans::PageOut::default();
    let mut lines = Vec::new();
    for story in parts.stories.iter().chain(&parts.footnotes) {
        paint_set(
            &story_set(layout, story, (page, &frame)),
            model,
            &layout.document,
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
