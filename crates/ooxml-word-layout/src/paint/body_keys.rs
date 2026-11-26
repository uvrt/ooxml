//! Owns: the keys of the paragraphs a page's body lines are painted for, in the order
//! its paint draws them ([`body_line_keys`]), read from the page's geometry without
//! painting it: what a comparison of two versions reads to know on which page each body
//! block is first drawn.
//!
//! Must not: paint, resolve a paragraph's formats or read a header, footer or note.

use super::{body_set, paragraph_of};
use crate::engine::DocumentLayout;
use crate::geometry::LineSource;
use ooxml_word_model::ids::BlockKey;

/// The paragraph key of each body line page `page` (0-based, blank pages counted) of
/// `layout` paints, in drawing order (`None`: a paragraph no story's index holds, such as a
/// separator the layout makes): the lines [`super::paint_page`] paints for its body (its
/// opening line and its stories' lines not among them; the endnotes placed in its flow
/// are). Empty past the last page.
pub fn body_line_keys(layout: &DocumentLayout, page: usize) -> Vec<Option<BlockKey>> {
    let Some(geometry) = layout.page_geometry(page) else {
        return Vec::new();
    };
    let frame = geometry.frame;
    let set = body_set(layout, &geometry, &frame);
    set.lines
        .iter()
        .filter(|line| !matches!(line.source, LineSource::Opening { .. }))
        .filter_map(|line| {
            let (paragraph, measured) = paragraph_of(&set, line)?;
            // `spans::paint_line` paints a line its paragraph broke into, none past them.
            measured.measured.lines.get(line.number)?;
            Some(paragraph.key)
        })
        .collect()
}
