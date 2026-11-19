//! Owns: the body's outline before and after a formatting edit, the after read again only
//! where the body's blocks changed: a block whose element is the one before keeps its id,
//! level and section break (a formatting edit adds or removes no block); a changed
//! paragraph's id and level are read again (`ooxml_word_model::docx::outline`). Anything
//! else changed -- a control, custom XML, a different number of blocks -- reads the whole
//! outline again.
//!
//! Must not: change anything.

use crate::names::W;
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::outline::{body_blocks, child_of, headings, Block, Heading, OutlineLevels};
use ooxml_word_model::docx::Docx;
use ooxml_xml::Node;
use std::sync::Arc;

/// The body's blocks as read before an edit.
pub(crate) struct Outline {
    blocks: Vec<Block>,
}

impl Outline {
    /// Read `docx`'s body.
    pub(crate) fn read(docx: &Docx) -> Outline {
        Outline {
            blocks: body_blocks(docx, &controls(docx)),
        }
    }

    /// The headings before.
    pub(crate) fn headings(&self) -> Vec<Heading> {
        headings(&self.blocks)
    }

    /// The headings of `docx`, the body after the edit.
    pub(crate) fn after(&self, docx: &Docx) -> Vec<Heading> {
        self.blocks_after(docx)
            .map_or_else(|| headings(&body_blocks(docx, &controls(docx))), |b| headings(&b))
    }

    fn blocks_after(&self, docx: &Docx) -> Option<Vec<Block>> {
        let body_part = docx.body()?;
        let (body_index, body) = child_of(body_part.root(), "body")?;
        let levels = OutlineLevels::new(docx);
        let mut out = Vec::with_capacity(self.blocks.len());
        let mut before = self.blocks.iter().enumerate();
        let mut section = 1;
        for (k, node) in body.children().iter().enumerate() {
            let Node::Element(node) = node else { continue };
            let name = node.name();
            let paragraph = name.is(Some(W), "p");
            let other = ["tbl", "sdt", "customXml"].iter().any(|l| name.is(Some(W), l));
            if !(paragraph || other) {
                continue;
            }
            let (j, was) = before.next()?;
            let next_section = self.blocks.get(j + 1).map_or(0, |n| n.section - was.section);
            if Arc::ptr_eq(&was.element, node) {
                out.push(Block { section, ..was.clone() });
                section += next_section;
                continue;
            }
            if !paragraph && !name.is(Some(W), "tbl") {
                return None;
            }
            let path = [body_index, u32::try_from(k).ok()?];
            let id = body_part
                .ids
                .paragraph_at(&path)
                .map(|p| p.id)
                .or_else(|| body_part.ids.table_at(&path).map(|t| t.id));
            let breaks = paragraph
                && node
                    .find(Some(W), "pPr")
                    .is_some_and(|p| p.find(Some(W), "sectPr").is_some());
            out.push(Block {
                id,
                level: paragraph.then(|| levels.effective(node)).flatten(),
                section,
                element: Arc::clone(node),
            });
            section += usize::from(breaks);
        }
        before.next().is_none().then_some(out)
    }
}
