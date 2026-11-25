//! Owns: naming what a layout draws at the edge ([`VersionNames`]): an [`Anchor`]'s
//! `data-docx-path` (the layout model's path: `w:body/w:tbl[2]/w:tr[3]/w:tc[1]/w:p[1]`,
//! `w:hdr/w:p[1]`) and its block's id, from the story parts' indexes -- the container
//! child's step from the segment tree's prefix counts in O(log n), the steps below it from
//! the block's place in its segment -- so a block is named when it is written out, not
//! carried by name through layout, paint and facts (phase 3 identity design, section 2).
//!
//! The paths are the ones the model writes today ([`super::cells::story_model_path`]); a
//! run's or a drawing's need its paragraph's projection, and are named there.
//!
//! Must not: key a memo, or keep a name past its version (a version's names are its own).

use super::cells::{level_places, path_below, story_model_path};
use super::{Docx, StoryPart};
use crate::ids::{Anchor, BlockId, BlockKey, BlockKind, StoryIx, StoryTable, Sub};
use ooxml_xml::Element;
use std::sync::Arc;

/// One version's story parts by story index: what its anchors are named by.
#[derive(Debug, Clone, Default)]
pub struct VersionNames {
    /// Each story's part, by index, sorted.
    parts: Vec<(StoryIx, Arc<StoryPart>)>,
}

impl VersionNames {
    /// The names of the version `docx`, its stories interned in its document's `stories`;
    /// a story past the table's limit is left out.
    #[must_use]
    pub fn new(docx: &Docx, stories: &mut StoryTable) -> VersionNames {
        VersionNames::from_parts(
            docx.parts
                .iter()
                .filter_map(|part| Some((stories.intern(&part.story)?, Arc::clone(part)))),
        )
    }

    /// The names of a version whose story parts are `parts`, each with its story's index (the
    /// first part of a story is taken).
    #[must_use]
    pub fn from_parts(parts: impl IntoIterator<Item = (StoryIx, Arc<StoryPart>)>) -> VersionNames {
        let mut parts: Vec<(StoryIx, Arc<StoryPart>)> = parts.into_iter().collect();
        parts.sort_by_key(|(ix, _)| *ix);
        parts.dedup_by_key(|(ix, _)| *ix);
        VersionNames { parts }
    }

    /// The part of the story `story`.
    #[must_use]
    pub fn part(&self, story: StoryIx) -> Option<&Arc<StoryPart>> {
        let k = self.parts.binary_search_by_key(&story, |(ix, _)| *ix).ok()?;
        self.parts.get(k).map(|(_, part)| part)
    }

    /// The id of the block `key` names.
    #[must_use]
    pub fn id(&self, key: BlockKey) -> Option<BlockId> {
        self.part(key.story())?.ids.id_of_key(key)
    }

    /// The layout model's path of `anchor` (`data-docx-path`); `None` where its block is
    /// not in this version, or for a run or drawing (named from its paragraph's projection).
    #[must_use]
    pub fn path(&self, anchor: Anchor) -> Option<String> {
        let part = self.part(anchor.block.story())?;
        let place = part.ids.place_of(anchor.block)?;
        let block = block_path(part, &place)?;
        match (anchor.sub, anchor.block.kind()) {
            (Sub::Whole, _) => Some(block),
            (Sub::Label, BlockKind::Paragraph) => Some(format!("{block}/label")),
            (Sub::Cell(row, cell), BlockKind::Table) => {
                let table = element_at(part.root(), &place)?;
                let rows = level_places(table, "tr");
                let row_element = rows.get(usize::from(row)).and_then(|below| element_at(table, below))?;
                (usize::from(cell) < level_places(row_element, "tc").len())
                    .then(|| format!("{block}/w:tr[{}]/w:tc[{}]", u32::from(row) + 1, u32::from(cell) + 1))
            }
            (Sub::LineNumbers(section), _) => Some(format!("w:sectPr[{}]/w:lnNumType", u64::from(section) + 1)),
            (Sub::Cols(section), _) => Some(format!("w:sectPr[{}]/w:cols", u64::from(section) + 1)),
            _ => None,
        }
    }

    /// `anchor`'s path and its block's id: what an output edge writes
    /// (`data-docx-path`, `data-docx-id`).
    #[must_use]
    pub fn name(&self, anchor: Anchor) -> Option<(String, BlockId)> {
        Some((self.path(anchor)?, self.id(anchor.block)?))
    }
}

/// The layout model's path of the block at `place` in `part`: the container child's step
/// from the index, the steps below it counted in that child; a block outside the container
/// (below the root, beside the body) by a walk from the root.
fn block_path(part: &StoryPart, place: &[u32]) -> Option<String> {
    let root = part.root();
    let container = part.ids.container();
    let c = container.len();
    if place.len() <= c || !place.starts_with(container) {
        return story_model_path(root, place);
    }
    let k = *place.get(c)?;
    let (local, count) = part.ids.container_step(k as usize)?;
    let (head, parent) = match container.first() {
        Some(body) => ("body", root.children().get(*body as usize)?.as_element()?),
        None => (root.name().local(), root),
    };
    let child = parent.children().get(k as usize)?.as_element()?;
    path_below(child, format!("w:{head}/w:{local}[{count}]"), place.get(c + 1..)?)
}

/// The element at child indexes `place` below `from`.
fn element_at<'a>(from: &'a Arc<Element>, place: &[u32]) -> Option<&'a Arc<Element>> {
    place
        .iter()
        .try_fold(from, |node, k| node.children().get(*k as usize)?.as_element())
}
