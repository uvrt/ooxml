//! Owns: naming what a layout draws at the edge ([`VersionNames`]): an [`Anchor`]'s
//! `data-docx-path` (the layout model's path: `w:body/w:tbl[2]/w:tr[3]/w:tc[1]/w:p[1]`,
//! `w:hdr/w:p[1]`) and its block's id, from the story parts' indexes -- the container
//! child's step from the segment tree's prefix counts in O(log n), the steps below it from
//! the block's place in its segment -- so a block is named when it is written out, not
//! carried by name through layout, paint and facts (phase 3 identity design, section 2).
//!
//! The paths are the ones the layout model writes today: [`super::cells::story_model_path`]'s,
//! but a note's blocks named from their note by its id (`w:footnote[@w:id=2]/w:p[1]`, as the
//! model reads a note as a story of its own) where `story_model_path` counts the notes
//! (`w:footnotes/w:footnote[3]/w:p[1]`). A run's or a drawing's need its paragraph's
//! projection, and are named there.
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

    /// The index of the story named `name` (`endnotes`), where this version has its part.
    #[must_use]
    pub fn story_named(&self, name: &str) -> Option<StoryIx> {
        self.parts
            .iter()
            .find(|(_, part)| part.story == name)
            .map(|(ix, _)| *ix)
    }

    /// The part of the story `story`.
    #[must_use]
    pub fn part(&self, story: StoryIx) -> Option<&Arc<StoryPart>> {
        let k = self.parts.binary_search_by_key(&story, |(ix, _)| *ix).ok()?;
        self.parts.get(k).map(|(_, part)| part)
    }

    /// Whether the block `key` names is in this version (without naming it).
    #[must_use]
    pub fn holds(&self, key: BlockKey) -> bool {
        self.part(key.story()).is_some_and(|part| match key.kind() {
            BlockKind::Row => part.ids.row_of(key).is_some(),
            BlockKind::Paragraph | BlockKind::Table => part.ids.entry_of(key).is_some(),
        })
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

    /// The blocks the paragraph or table `paragraph` is in, outermost first, ending with
    /// it: each table, row and cell between its story's block container (a note, in a notes
    /// part) and it (a cell as its table's [`Sub::Cell`]), wrappers passed through -- what a
    /// page draws something of where it draws a line of the paragraph (`paths.block_keys`).
    /// `None` where the key is not a paragraph's or table's of this version, or where
    /// something other than a table, row, cell or wrapper stands between (a text box's
    /// paragraph).
    #[must_use]
    pub fn blocks_of(&self, paragraph: BlockKey) -> Option<Vec<Anchor>> {
        if !matches!(paragraph.kind(), BlockKind::Paragraph | BlockKind::Table) {
            return None;
        }
        let story = paragraph.story();
        let part = self.part(story)?;
        let ids = &part.ids;
        let place = ids.place_of(paragraph)?;
        let container = ids.container();
        if !place.starts_with(container) {
            return None;
        }
        // a note's blocks are below their note: the story's container is each note
        let c = container.len() + usize::from(matches!(part.root().name().local(), "footnotes" | "endnotes"));
        let mut node = element_at(part.root(), place.get(..c)?)?;
        let mut out = Vec::new();
        // the innermost table (its key, element and place) and row (its place in it)
        let mut table: Option<(BlockKey, &Arc<Element>, usize)> = None;
        let mut row: Option<(&Arc<Element>, usize, u32)> = None;
        for depth in c..place.len() {
            node = node.children().get(*place.get(depth)? as usize)?.as_element()?;
            let here = place.get(..=depth)?;
            match node.name().local() {
                "tbl" => {
                    let key = ids.key_of(story, ids.at(here)?)?;
                    out.push(Anchor::whole(key));
                    if depth + 1 == place.len() {
                        return (key == paragraph).then_some(out);
                    }
                    table = Some((key, node, depth + 1));
                }
                "tr" => {
                    let (_, element, from) = table?;
                    let r = position(element, "tr", here.get(from..)?)?;
                    let crate::ids::Entry::Table(n) = ids.at(place.get(..from)?)? else {
                        return None;
                    };
                    out.push(Anchor::whole(ids.row_key_of(story, n, r)?));
                    row = Some((node, depth + 1, u32::try_from(r).ok()?));
                }
                "tc" => {
                    let ((key, ..), (element, from, r)) = (table?, row?);
                    let cell = position(element, "tc", here.get(from..)?)?;
                    out.push(Anchor {
                        block: key,
                        sub: Sub::Cell(u16::try_from(r).ok()?, u16::try_from(cell).ok()?),
                    });
                }
                "p" => {
                    out.push(Anchor::whole(paragraph));
                    return (depth + 1 == place.len()).then_some(out);
                }
                "sdt" | "sdtContent" | "customXml" => {}
                _ => return None,
            }
        }
        None
    }

    /// `anchor`'s path and its block's id: what an output edge writes
    /// (`data-docx-path`, `data-docx-id`).
    #[must_use]
    pub fn name(&self, anchor: Anchor) -> Option<(String, BlockId)> {
        Some((self.path(anchor)?, self.id(anchor.block)?))
    }

    /// The key of the paragraph, table or row the layout model's path `path` names in the
    /// story `story` (a path an edge is handed: [`StoryPart::resolve_path`]'s rules); `None`
    /// where it names no block of the story's own, or one inside a paragraph's content (a
    /// text box's).
    #[must_use]
    pub fn key_at_path(&self, story: StoryIx, path: &str) -> Option<BlockKey> {
        let part = self.part(story)?;
        let (_, place) = part.resolve_path(path)?;
        let mut node = part.root();
        for k in place.iter().take(place.len().saturating_sub(1)) {
            node = node.children().get(*k as usize)?.as_element()?;
            if node.name().local() == "p" {
                return None;
            }
        }
        part.ids.key_at(story, &place)
    }

    /// The key of the paragraph, table or row the tool's id `id` names in the story `story`,
    /// as written (no alias followed).
    #[must_use]
    pub fn key_of_id(&self, story: StoryIx, id: &str) -> Option<BlockKey> {
        let ids = &self.part(story)?.ids;
        if let Some(entry) = ids.get(id) {
            return ids.key_of(story, entry);
        }
        // a row: by its own paraId's id, or by its table's and its place
        let place = ids
            .tables()
            .iter()
            .flat_map(|t| t.rows.into_iter())
            .filter(|(_, row)| row == id)
            .map(|(place, _)| place)
            .min()?;
        ids.key_at(story, &place)
    }

    /// The top-level block (the block container's child's, wrappers passed through: a
    /// paragraph or a table) the paragraph `paragraph` is in, itself where it is one; `None`
    /// as for [`VersionNames::blocks_of`].
    #[must_use]
    pub fn top_block_of(&self, paragraph: BlockKey) -> Option<BlockKey> {
        self.blocks_of(paragraph)?.first().map(|a| a.block)
    }

    /// The place (child indexes from its part's root) of the block `key` names.
    #[must_use]
    pub fn place_of(&self, key: BlockKey) -> Option<crate::ids::Path> {
        self.part(key.story())?.ids.place_of(key)
    }
}

/// The layout model's path of the block at `place` in `part`: the container child's step
/// from the index, the steps below it counted in that child; a block outside the container
/// (below the root, beside the body) by a walk from the root.
pub(crate) fn block_path(part: &StoryPart, place: &[u32]) -> Option<String> {
    let root = part.root();
    if let Some((host, content)) = text_box_of(root, place) {
        // a text box's block: below its paragraph's path, as the drawing readers name it
        let host_path = block_path(part, place.get(..=host)?)?;
        let paragraph = element_at(root, place.get(..=host)?)?;
        let steps = crate::read::paths::text_box_path(paragraph, place.get(host + 1..=content)?)?;
        let content_element = element_at(root, place.get(..=content)?)?;
        return path_below(content_element, format!("{host_path}/{steps}"), place.get(content + 1..)?);
    }
    if let Some(path) = note_path(root, place) {
        return path;
    }
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

/// The layout model's path of the element at `place` in a notes part whose root is `root`,
/// from its note (`w:footnote[@w:id=2]/w:p[1]`: a note's id as written, `None` where it has
/// none); `None` (not a note's) where `root` is not a notes part.
fn note_path(root: &Arc<Element>, place: &[u32]) -> Option<Option<String>> {
    let kind = match root.name().local() {
        "footnotes" => "footnote",
        "endnotes" => "endnote",
        _ => return None,
    };
    let (first, below) = place.split_first()?;
    let note = root.children().get(*first as usize)?.as_element()?;
    if note.name().local() != kind {
        return Some(None);
    }
    let id = crate::w::Note::new(note).id().unwrap_or("None");
    Some(path_below(note, format!("w:{kind}[@w:id={id}]"), below))
}

/// The place among `parent`'s rows or cells (`level`, counted through wrappers) of the one
/// at `below` (child indexes from `parent`).
fn position(parent: &Element, level: &str, below: &[u32]) -> Option<usize> {
    level_places(parent, level).iter().position(|p| p.as_slice() == below)
}

/// The element at child indexes `place` below `from`.
fn element_at<'a>(from: &'a Arc<Element>, place: &[u32]) -> Option<&'a Arc<Element>> {
    place
        .iter()
        .try_fold(from, |node, k| node.children().get(*k as usize)?.as_element())
}

/// Where the block at `place` is in a text box: the depth (an index into `place`) of the
/// paragraph holding the innermost text box it is in and of that text box's content
/// (`w:txbxContent`); `None` for a block in no text box.
fn text_box_of(root: &Arc<Element>, place: &[u32]) -> Option<(usize, usize)> {
    let mut node: &Element = root;
    let (mut host, mut content) = (None, None);
    for (depth, k) in place.iter().enumerate() {
        node = node.children().get(*k as usize)?.as_element()?;
        match node.name().local() {
            "p" if depth + 1 < place.len() => host = Some(depth),
            "txbxContent" => content = host.map(|h| (h, depth)),
            _ => {}
        }
    }
    content
}
