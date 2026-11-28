//! Owns: one top-level block of a story part projected on its own ([`Projection::block`]),
//! as a [`BlockAlone`]: read through the same reader, keyed from the part's index and kept
//! in the same block tier as a full projection's body elements, but in a context it assumes
//! ([`Assumed`]): no field open at it, no paragraph before it joined into it, no properties
//! from the paragraph after it. Those change its paragraphs' runs and properties, so a
//! `BlockAlone` gives none of them: only its cells' facts -- the cell of a paragraph (the
//! table style and regions), the cells' resolved properties, a table's look and style --
//! which none of them changes (`tests/integration/block_alone.rs` holds it to the full
//! projection on the corpus).
//!
//! Must not: give a paragraph's runs or properties, or be taken for a story's reading.

use super::{memo, Keyer, Parts, Projection};
use crate::docx::formatting::Cell;
use crate::docx::{at_path, Docx};
use crate::error::WordError;
use crate::ids::BlockKey;
use crate::model::{Block, Graphic, InlineObject, Look, Paragraph, Table};
use crate::read::{At, Fields, Reader};
use crate::resolve::{resolve_table, ResolvedTable, Sources};
use crate::value::{Props, Text};
use crate::view::View;
use crate::xmlread::Ns;
use ooxml_xml::Element;
use std::sync::Arc;

/// What a block projected alone assumes of its place in its story: no field open at it,
/// nothing joined into it from the paragraph before it, no properties taken from the
/// paragraph after it. Where its story differs, its paragraphs' runs and properties differ
/// from the story's projection; its cells' facts do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assumed;

/// One top-level block of a story part projected alone: its cells' facts.
#[derive(Debug, Clone)]
pub struct BlockAlone {
    /// The blocks the element reads as (a table parted by content moved out of it is
    /// several): crate-private, as their paragraphs' runs and properties are out of
    /// context ([`Assumed`]).
    blocks: Arc<[Block]>,
}

/// A table of a [`BlockAlone`]: its structure and cells, not its paragraphs.
#[derive(Debug, Clone, Copy)]
pub struct TableAlone<'a> {
    table: &'a Table,
}

impl BlockAlone {
    /// What it assumed of its place.
    #[must_use]
    pub fn assumed(&self) -> Assumed {
        Assumed
    }

    /// The cell of the paragraph `paragraph` (by its key), in the table holding it (a
    /// nested table's own, or one in a text box's); `None` outside the block's tables.
    #[must_use]
    pub fn cell(&self, paragraph: BlockKey) -> Option<Cell> {
        let mut found = None;
        visit(&self.blocks, &mut |block| {
            if let Block::Paragraph(p) = block {
                if p.key == Some(paragraph) {
                    found = found.take().or_else(|| {
                        p.table_style_id.as_ref().map(|style| Cell {
                            style: style.clone(),
                            conditions: Arc::clone(&p.table_conditions),
                        })
                    });
                }
            }
        });
        found
    }

    /// Its tables, nested ones and those in text boxes after the block holding them, in
    /// document order.
    #[must_use]
    pub fn tables(&self) -> Vec<TableAlone<'_>> {
        let mut out = Vec::new();
        collect_tables(&self.blocks, &mut out);
        out
    }

    /// The table keyed `key` (its first part, where the view parts it), nested or not.
    #[must_use]
    pub fn table(&self, key: BlockKey) -> Option<TableAlone<'_>> {
        self.tables().into_iter().find(|t| t.table.key == Some(key))
    }

    /// Its blocks, for this crate's own tests of it against a full projection.
    #[cfg(test)]
    pub(crate) fn blocks(&self) -> &[Block] {
        &self.blocks
    }
}

/// Every block of `blocks`, those in tables' cells and text boxes included, in document
/// order.
fn visit<'a>(blocks: &'a [Block], f: &mut dyn FnMut(&'a Block)) {
    for block in blocks {
        f(block);
        match block {
            Block::Table(t) => {
                for cell in t.rows.iter().flat_map(|r| r.iter()) {
                    visit(cell, f);
                }
            }
            Block::Paragraph(p) => {
                for graphic in text_boxes(p) {
                    visit_graphic(graphic, f);
                }
            }
        }
    }
}

fn visit_graphic<'a>(graphic: &'a Graphic, f: &mut dyn FnMut(&'a Block)) {
    visit(&graphic.text, f);
    for child in graphic.children.iter() {
        visit_graphic(child, f);
    }
}

/// The graphics (shapes, groups) of a paragraph's runs, inline and anchored.
fn text_boxes(p: &Paragraph) -> impl Iterator<Item = &Graphic> {
    p.runs.iter().flat_map(|run| {
        let inline = run.drawings.iter().filter_map(|d| match d {
            InlineObject::Graphic(g) => Some(&**g),
            _ => None,
        });
        inline.chain(run.anchors.iter().filter_map(|a| a.graphic.as_deref()))
    })
}

fn collect_tables<'a>(blocks: &'a [Block], out: &mut Vec<TableAlone<'a>>) {
    visit(blocks, &mut |block| {
        if let Block::Table(table) = block {
            out.push(TableAlone { table });
        }
    });
}

impl TableAlone<'_> {
    /// Its key in its story.
    #[must_use]
    pub fn key(&self) -> Option<BlockKey> {
        self.table.key
    }

    /// `w:tblStyle`.
    #[must_use]
    pub fn style_id(&self) -> Option<&Text> {
        self.table.style_id.as_ref()
    }

    /// `w:tblLook`.
    #[must_use]
    pub fn look(&self) -> Look {
        self.table.look
    }

    /// Resolved against `sources`: its rows and cells as the view keeps them.
    #[must_use]
    pub fn resolved(&self, sources: &Sources<'_>) -> ResolvedTable {
        resolve_table(sources, self.table)
    }

    /// The view's row and cell of the cell whose place as written is `(row, cell)` (its row
    /// among the table's rows, its own among the row's cells, wrappers seen through).
    #[must_use]
    pub fn cell_at(&self, row: u32, cell: u32) -> Option<(usize, usize)> {
        self.table
            .cell_places
            .iter()
            .enumerate()
            .find_map(|(r, cells)| cells.iter().position(|place| *place == (row, cell)).map(|c| (r, c)))
    }

    /// The resolved properties of the cell whose place as written is `(row, cell)`.
    #[must_use]
    pub fn cell_properties(&self, sources: &Sources<'_>, row: u32, cell: u32) -> Option<Props> {
        let (r, c) = self.cell_at(row, cell)?;
        let resolved = self.resolved(sources);
        let found = resolved.rows.into_iter().nth(r)?.cells.into_iter().nth(c)?;
        Some(Arc::unwrap_or_clone(found).properties)
    }
}

/// The place of the top-level block holding the element at `place` in a story part whose
/// root is `root`: the first `w:p`, `w:tbl`, `w:sdt` or `w:customXml` on the way down.
#[must_use]
pub fn block_place(root: &Arc<Element>, place: &[u32]) -> Option<Vec<u32>> {
    (1..=place.len()).find_map(|n| {
        let prefix = place.get(..n)?;
        let element = at_path(root, prefix)?;
        ["p", "tbl", "sdt", "customXml"]
            .iter()
            .any(|l| Ns::W.named(element, l))
            .then(|| prefix.to_vec())
    })
}

impl Projection {
    /// The top-level block of the story part `part` at `place` (child indexes from its
    /// root; [`block_place`]) projected alone in `view`: from the block tier where the very
    /// element was read so before, else read and kept. `None` where `part` is not a story
    /// part with an index, or nothing is at `place`.
    ///
    /// # Errors
    /// [`WordError::NoMainPart`] where the version has no main part's tree; a part not XML.
    pub fn block(version: &Docx, part: &str, place: &[u32], view: View) -> Result<Option<BlockAlone>, WordError> {
        let Some(story) = version.part(part) else {
            return Ok(None);
        };
        let Some(ix) = story.index else { return Ok(None) };
        let Some(element) = at_path(story.root(), place) else {
            return Ok(None);
        };
        let parts = Parts::of(version)?;
        let projections = version.kept().projections();
        let context = projections.context([
            parts.theme.as_ref(),
            parts.settings.as_ref(),
            parts.styles.as_ref(),
            parts.numbering.as_ref().map(|(_, t)| t),
        ]);
        let colours = Arc::clone(&context.colours);
        let of = memo::ElementOf {
            view,
            story: ix,
            colours: &colours,
            open: &[],
        };
        if let Some(kept) = projections.element(element, &of) {
            return Ok(Some(BlockAlone {
                blocks: Arc::clone(&kept.blocks),
            }));
        }
        let keyer = Keyer::new(ix, story);
        let at = At::Keyed(&keyer, place.to_vec());
        // a story's own fields: the body's draw cross references and STYLEREF, a header's
        // STYLEREF, a note's neither (as the full projection reads each)
        let mut fields = match story.story.as_str() {
            "body" => Fields::new(true, true),
            s if s.starts_with("header") || s.starts_with("footer") => Fields::new(false, true),
            _ => Fields::new(false, false),
        };
        let read = Arc::new(Reader::new(&colours, view).body_element(element, &at, &mut fields));
        if fields.stack.is_empty() {
            projections.keep_element(element, &of, Arc::clone(&read));
        }
        Ok(Some(BlockAlone {
            blocks: Arc::clone(&read.blocks),
        }))
    }
}

#[cfg(test)]
mod tests;
