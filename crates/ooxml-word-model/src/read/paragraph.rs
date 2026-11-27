//! Owns: a `w:p` as a [`Paragraph`] -- its runs and bookmarks, its properties, its table
//! context, its key (or path) and id, whether its mark is deleted, the section it ends.
//!
//! Must not: join paragraphs (`join`), or read runs (`runs`).

use super::fields::Fields;
use super::{At, KeyOf, Reader, Steps};
use crate::model::{Block, Graphic, InlineObject, Paragraph, Run, Table};
use crate::value::{text, Text};
use crate::w;
use crate::xmlread::child;
use ooxml_xml::Element;
use std::sync::Arc;

/// Whether a paragraph's mark is deleted, or moved away, in the final view.
pub(crate) fn mark_removed(element: &Element) -> bool {
    child(element, "pPr")
        .and_then(|p| child(p, "rPr"))
        .is_some_and(|m| child(m, "del").is_some() || child(m, "moveFrom").is_some())
}

impl Reader<'_> {
    /// The paragraph `element`, in a table with `table_style` and `conditions` where it is
    /// in a cell.
    pub(crate) fn paragraph(
        &self,
        element: &Element,
        table: Option<(&Text, &Arc<[Text]>)>,
        at: &At<'_>,
        fields: &mut Fields,
    ) -> Paragraph {
        let (key, unkeyed) = at.name();
        let mut runs = Vec::new();
        let outer = fields.bookmarks.replace(Vec::new());
        self.inline(element, &mut Steps::new(""), fields, &mut runs);
        if let At::Keyed(keys, place) = at {
            key_text_boxes(&mut runs, element, place, *keys);
        }
        let bookmarks = std::mem::replace(&mut fields.bookmarks, outer).unwrap_or_default();
        let ppr = child(element, "pPr");
        let section = ppr
            .and_then(|p| child(p, "sectPr"))
            .map(|s| Arc::new(super::section_props(s)));
        Paragraph {
            runs: Arc::from(runs),
            properties: self.para_props(ppr),
            table_style_id: table.map(|(s, _)| s.clone()),
            table_conditions: table.map_or_else(|| Arc::from(Vec::new()), |(_, c)| Arc::clone(c)),
            key,
            unkeyed,
            para_id: w::P::new(element).para_id().map(text),
            joins: Arc::from(Vec::new()),
            mark_deleted: mark_removed(element),
            section,
            bookmarks: Arc::from(bookmarks),
        }
    }
}

/// `runs` (a paragraph's, at `place` in its part, the element `paragraph`) with the blocks of
/// their text boxes keyed from the story's index: each block, read by its path below the
/// paragraph, at the place that path names (phase 3 identity design, RD1: the branch of an
/// `mc:AlternateContent` the index keys is the one the reader reads, `crate::mc`). A block
/// the index does not key keeps its path.
fn key_text_boxes(runs: &mut [Run], paragraph: &Element, place: &[u32], keys: &dyn KeyOf) {
    let host = Host { paragraph, place, keys };
    for run in runs.iter_mut() {
        if run.drawings.iter().any(|d| matches!(d, InlineObject::Graphic(_))) {
            let drawings: Vec<InlineObject> = run
                .drawings
                .iter()
                .map(|d| match d {
                    InlineObject::Graphic(g) => InlineObject::Graphic(host.graphic(g)),
                    other => other.clone(),
                })
                .collect();
            run.drawings = Arc::from(drawings);
        }
        if run.anchors.iter().any(|a| a.graphic.is_some()) {
            let anchors: Vec<crate::model::Anchor> = run
                .anchors
                .iter()
                .map(|a| crate::model::Anchor {
                    graphic: a.graphic.as_ref().map(|g| host.graphic(g)),
                    ..a.clone()
                })
                .collect();
            run.anchors = Arc::from(anchors);
        }
    }
}

/// The paragraph a text box's blocks are read below, and what keys them.
struct Host<'a> {
    paragraph: &'a Element,
    place: &'a [u32],
    keys: &'a dyn KeyOf,
}

impl Host<'_> {
    /// `graphic` with its text box's blocks, and its members', keyed.
    fn graphic(&self, graphic: &Arc<Graphic>) -> Arc<Graphic> {
        if graphic.text.is_empty() && graphic.children.is_empty() {
            return Arc::clone(graphic);
        }
        Arc::new(Graphic {
            text: graphic.text.iter().map(|b| self.block(b)).collect(),
            children: graphic.children.iter().map(|g| self.graphic(g)).collect(),
            ..(**graphic).clone()
        })
    }

    /// The place in the part of a block read at `path` below the paragraph, and its element.
    fn found(&self, path: Option<&crate::value::Text>) -> Option<(Vec<u32>, &Element)> {
        let below = super::paths::place_below_path(self.paragraph, path?)?;
        let element = element_at(self.paragraph, &below)?;
        let mut place = self.place.to_vec();
        place.extend(below);
        Some((place, element))
    }

    fn block(&self, block: &Block) -> Block {
        match block {
            Block::Paragraph(p) => {
                let Some((place, element)) = self.found(p.unkeyed.as_ref()).filter(|_| p.key.is_none()) else {
                    return block.clone();
                };
                let Some(key) = self.keys.key_at(&place) else {
                    return block.clone();
                };
                let mut runs = p.runs.to_vec();
                key_text_boxes(&mut runs, element, &place, self.keys);
                Block::Paragraph(Arc::new(Paragraph {
                    key: Some(key),
                    unkeyed: None,
                    runs: Arc::from(runs),
                    ..(**p).clone()
                }))
            }
            Block::Table(t) => {
                let rows = t
                    .rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|cell| cell.iter().map(|b| self.block(b)).collect())
                            .collect()
                    })
                    .collect();
                let key = self
                    .found(t.unkeyed.as_ref())
                    .filter(|_| t.key.is_none())
                    .and_then(|(place, _)| self.keys.key_at(&place));
                Block::Table(Arc::new(match key {
                    Some(key) => Table {
                        key: Some(key),
                        unkeyed: None,
                        unkeyed_cells: None,
                        rows,
                        ..(**t).clone()
                    },
                    None => Table { rows, ..(**t).clone() },
                }))
            }
        }
    }
}

/// The element at child indexes `at` below `element`.
fn element_at<'a>(mut element: &'a Element, at: &[u32]) -> Option<&'a Element> {
    for k in at {
        element = element.children().get(*k as usize)?.as_element()?;
    }
    Some(element)
}
