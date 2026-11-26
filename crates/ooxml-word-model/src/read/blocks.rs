//! Owns: a container's blocks (a part's body, a cell, a content control's content, a note,
//! a text box) in document order and in the final view, and a story's blocks with the
//! properties a deleted mark takes from what follows -- docx2svg's `_block_list`,
//! `_block_of`, `_blocks`, `_story` and `_text_box_story`; and one top-level element of the
//! body read on its own ([`BodyElement`]), the unit a reopened version reuses.
//!
//! Must not: decide which body elements are reused (`crate::document`).

use super::fields::Fields;
use super::join::{following_properties, join};
use super::{At, Reader, Steps};
use crate::model::Block;
use crate::value::Text;
use crate::xmlread::local;
use ooxml_xml::Element;
use std::sync::Arc;

/// What one top-level element of the body reads as: its blocks (before the final view's
/// joins across elements) and the keywords of the fields it draws as cached.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BodyElement {
    /// Its blocks.
    pub blocks: Arc<[Block]>,
    /// The fields it draws as cached.
    pub cached: Arc<[Text]>,
}

impl Reader<'_> {
    /// The blocks of a container's elements, in order (not yet joined).
    pub(crate) fn block_list(
        &self,
        parent: &Element,
        table: Option<(&Text, &Arc<[Text]>)>,
        at: &At<'_>,
        fields: &mut Fields,
    ) -> Vec<Block> {
        let mut out = Vec::new();
        let mut steps = Steps::new(match at {
            At::Path(path) => path,
            At::Keyed(..) => "",
        });
        for (k, node) in parent.children().iter().enumerate() {
            let Some(node) = node.as_element() else { continue };
            let here = at.child(k, node, &mut steps);
            self.block_of(node, &here, table, fields, &mut out);
        }
        out
    }

    /// The blocks of one of a container's elements into `out`.
    pub(crate) fn block_of(
        &self,
        node: &Element,
        here: &At<'_>,
        table: Option<(&Text, &Arc<[Text]>)>,
        fields: &mut Fields,
        out: &mut Vec<Block>,
    ) {
        match local(node) {
            "p" => out.push(Block::Paragraph(Arc::new(self.paragraph(node, table, here, fields)))),
            "tbl" => out.extend(self.table_blocks(node, here, fields)),
            "sdt" => {
                let content = node
                    .children()
                    .iter()
                    .enumerate()
                    .find_map(|(k, n)| n.as_element().filter(|e| local(e) == "sdtContent").map(|e| (k, e)));
                if let Some((k, content)) = content {
                    out.extend(self.block_list(content, table, &here.content(k), fields));
                }
            }
            "customXml" => out.extend(self.block_list(node, table, here, fields)),
            _ => {}
        }
    }

    /// One top-level body element read on its own, with the fields open at it.
    pub(crate) fn body_element(&self, node: &Element, here: &At<'_>, fields: &mut Fields) -> BodyElement {
        let start = fields.cached.len();
        let mut out = Vec::new();
        self.block_of(node, here, None, fields, &mut out);
        let cached = fields.cached.get(start..).unwrap_or_default().to_vec();
        BodyElement {
            blocks: Arc::from(out),
            cached: Arc::from(cached),
        }
    }

    /// A story's blocks (a part's, a note's, a text box's) in the final view.
    pub(crate) fn story(&self, parent: &Element, at: &At<'_>, fields: &mut Fields) -> Vec<Block> {
        following_properties(join(self.block_list(parent, None, at, fields)), None).0
    }

    /// A text box's blocks: the part's fields, but no cross reference or `STYLEREF`
    /// computed in it.
    pub(crate) fn text_box_story(&self, parent: &Element, path: &str, fields: &mut Fields) -> Vec<Block> {
        let outer = (fields.cross_references, fields.styleref);
        fields.cross_references = false;
        fields.styleref = false;
        let out = self.story(parent, &At::Path(crate::value::text(path)), fields);
        (fields.cross_references, fields.styleref) = outer;
        out
    }
}

/// The final view of the body from its top-level elements' blocks.
pub fn body_story(elements: impl IntoIterator<Item = Block>) -> Vec<Block> {
    following_properties(join(elements.into_iter().collect()), None).0
}
