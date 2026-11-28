//! Owns: what the layout model reads of a table's cells ([`TableFacts`]): its top-level
//! block projected alone ([`Projection::block`], the block tier's, kept across a document's
//! versions by the element read), each paragraph's cell (table style and regions) by the
//! paragraph's key, each cell's resolved properties (its shading as drawn), the tables'
//! looks and styles; and the layout model's structural path of an element of a story part
//! ([`story_model_path`]: a field's run, as the layout names what it draws).
//!
//! Must not: give a table's paragraphs' runs or properties (a block alone is read out of
//! its story's context: [`crate::projection::Assumed`]).

use super::formatting::{Cell, Formatting};
use super::{Docx, StoryPart};
use crate::ids::{BlockKey, W};
use crate::projection::{block_place, BlockAlone, Projection, TableAlone};
use crate::resolve::{resolve_table, ResolvedTable};
use crate::view::View;
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// The key of the block (paragraph, table) at `place` of `part`, from its index.
#[must_use]
pub fn key_at(part: &StoryPart, place: &[u32]) -> Option<BlockKey> {
    part.ids.key_of(part.index?, part.ids.at(place)?)
}

/// What the layout model reads of one table's cells: the top-level block holding it,
/// projected alone in the final view.
#[derive(Debug, Clone)]
pub struct TableFacts {
    block: BlockAlone,
    table: Option<BlockKey>,
}

impl TableFacts {
    /// The facts of the table at `place` of the story part `part` (the table itself, or
    /// one in a cell or text box of a top-level block); `None` where the part has no index
    /// or nothing is there.
    #[must_use]
    pub fn of(docx: &Docx, part: &str, place: &[u32]) -> Option<TableFacts> {
        let story = docx.part(part)?;
        let top = block_place(story.root(), place)?;
        let block = Projection::block(docx, part, &top, View::Final).ok()??;
        Some(TableFacts {
            block,
            table: key_at(story, place),
        })
    }

    /// The cell of the paragraph keyed `paragraph`.
    #[must_use]
    pub fn cell(&self, paragraph: BlockKey) -> Option<Cell> {
        self.block.cell(paragraph)
    }

    /// The cell of the paragraph at `place` of `part` (the part the facts were read from).
    #[must_use]
    pub fn cell_at(&self, part: &StoryPart, place: &[u32]) -> Option<Cell> {
        self.cell(key_at(part, place)?)
    }

    /// The table asked for.
    #[must_use]
    pub fn table(&self) -> Option<TableAlone<'_>> {
        self.block.table(self.table?)
    }

    /// The table keyed `key` in the same top-level block (a nested one).
    #[must_use]
    pub fn table_keyed(&self, key: BlockKey) -> Option<TableAlone<'_>> {
        self.block.table(key)
    }
}

/// The resolved formats of the `w:tbl` `element`, a table in no reading (one an edit is
/// writing), from its structure in the final view alone (`crate::docx::tables`, its cells
/// unread): what [`TableAlone::resolved`] gives a table of a version (held to it on the
/// corpus, `tests/integration/block_alone.rs`).
#[must_use]
pub fn resolved_shape(formatting: &Formatting, element: &Element) -> Option<ResolvedTable> {
    let shape = super::tables::table_shape(View::Final, &formatting.context().colours, element)?;
    Some(resolve_table(&formatting.sources(), &shape))
}

/// The cells of one part's paragraphs as the layout model reads them: each top-level block
/// holding a paragraph asked about projected once ([`Projection::block`]), the paragraph's
/// cell found by its key.
pub struct PartCells<'a> {
    docx: &'a Docx,
    part: &'a StoryPart,
    blocks: HashMap<Vec<u32>, Option<BlockAlone>>,
}

impl<'a> PartCells<'a> {
    /// The cells of `story`'s paragraphs, `story` a story part of `docx`.
    #[must_use]
    pub fn of(docx: &'a Docx, story: &'a StoryPart) -> PartCells<'a> {
        PartCells {
            docx,
            part: story,
            blocks: HashMap::new(),
        }
    }

    /// The cell of the paragraph at `place` (child indexes from the root); `None` outside
    /// tables.
    pub fn cell(&mut self, place: &[u32]) -> Option<Cell> {
        let in_table = (1..place.len()).any(|d| {
            place
                .get(..d)
                .and_then(|prefix| super::at_path(self.part.root(), prefix))
                .is_some_and(|e| e.name().is(Some(W), "tbl"))
        });
        if !in_table {
            return None;
        }
        let key = key_at(self.part, place)?;
        let top = block_place(self.part.root(), place)?;
        let (docx, part) = (self.docx, self.part);
        let block = self
            .blocks
            .entry(top)
            .or_insert_with_key(|top| Projection::block(docx, &part.part, top, View::Final).ok().flatten());
        block.as_ref()?.cell(key)
    }
}

/// The places of each parent's rows or cells (wrappers looked into), by the parent's
/// address and the level: a table's or row's found once for all the paragraphs in it.
#[derive(Default)]
struct Levels(HashMap<(usize, &'static str), HashMap<Vec<u32>, usize>>);

impl Levels {
    /// The place among `parent`'s rows or cells (`level`) of the one at `path` below it.
    fn index(&mut self, parent: &Element, level: &'static str, path: &[u32]) -> Option<usize> {
        let found = self
            .0
            .entry((std::ptr::from_ref(parent) as usize, level))
            .or_insert_with(|| {
                let mut found = Vec::new();
                level_paths(parent, level, &mut Vec::new(), &mut found);
                found.into_iter().enumerate().map(|(k, p)| (p, k)).collect()
            });
        found.get(path).copied()
    }
}

/// The layout model's path of the element at child indexes `place` in the story part whose
/// root is `root`, as the model names a story's blocks and runs: the main part's from its
/// body (`w:body/w:p[3]/w:r[2]`), any other part's from its root (`w:hdr/w:p[1]/w:r[1]`);
/// rows and cells counted among a table's rows and a row's cells wherever wrappers (content
/// controls, custom XML) hold them.
#[must_use]
pub fn story_model_path(root: &Arc<Element>, place: &[u32]) -> Option<String> {
    let (first, rest) = place.split_first()?;
    match root.children().get(*first as usize)?.as_element() {
        Some(body) if body.name().is(Some(W), "body") => {
            path_from(body, String::from("w:body"), rest, &mut Levels::default())
        }
        _ => path_from(
            root,
            format!("w:{}", root.name().local()),
            place,
            &mut Levels::default(),
        ),
    }
}

/// The layout model's path of the element at `below` under `start`, whose own path is
/// `prefix` (the part's index names a container child's step; the steps below it are
/// counted here, rows and cells as [`story_model_path`] counts them).
pub(crate) fn path_below(start: &Arc<Element>, prefix: String, below: &[u32]) -> Option<String> {
    path_from(start, prefix, below, &mut Levels::default())
}

/// The places below `parent` (child indexes) of its rows or cells (`level`), counted through
/// row- and cell-level wrappers as [`story_model_path`] counts them, in order.
pub(crate) fn level_places(parent: &Element, level: &str) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    level_paths(parent, level, &mut Vec::new(), &mut out);
    out
}

/// The steps from `start` to the element at `below`, after `prefix`.
fn path_from(start: &Arc<Element>, prefix: String, below: &[u32], levels: &mut Levels) -> Option<String> {
    let mut out = prefix;
    let mut node = Arc::clone(start);
    let mut steps = below.iter().peekable();
    while let Some(k) = steps.next() {
        let parent_name = node.name().local().to_owned();
        let child = node.children().get(*k as usize)?.as_element()?.clone();
        let name = child.name().local().to_owned();
        let level = match parent_name.as_str() {
            "tbl" => Some("tr"),
            "tr" => Some("tc"),
            _ => None,
        };
        if let Some(level) = level.filter(|l| name != *l) {
            // A wrapper at row or cell level: the row or cell inside it, counted among all.
            let mut path = vec![*k];
            let mut inner = Arc::clone(&child);
            while inner.name().local() != level {
                let step = *steps.next()?;
                path.push(step);
                inner = inner.children().get(step as usize)?.as_element()?.clone();
            }
            let index = levels.index(&node, level, &path)?;
            out.push_str(&format!("/w:{level}[{}]", index + 1));
            node = inner;
            continue;
        }
        if let Some(level) = level {
            let index = levels.index(&node, level, &[*k])?;
            out.push_str(&format!("/w:{level}[{}]", index + 1));
        } else {
            let count = node
                .children()
                .iter()
                .take(*k as usize)
                .filter(|n| matches!(n, Node::Element(e) if e.name().local() == name))
                .count();
            out.push_str(&format!("/w:{name}[{}]", count + 1));
        }
        node = child;
    }
    Some(out)
}

fn level_paths(parent: &Element, level: &str, path: &mut Vec<u32>, out: &mut Vec<Vec<u32>>) {
    for (k, node) in parent.children().iter().enumerate() {
        let Node::Element(e) = node else { continue };
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        let local = e.name().local();
        if e.name().namespace() == Some(W) {
            if local == level {
                out.push(path.clone());
            } else if local == "sdt" {
                if let Some(j) = e
                    .children()
                    .iter()
                    .position(|n| matches!(n, Node::Element(c) if c.name().is(Some(W), "sdtContent")))
                {
                    path.push(u32::try_from(j).unwrap_or(u32::MAX));
                    if let Some(content) = e.children().get(j).and_then(Node::as_element) {
                        level_paths(content, level, path, out);
                    }
                    path.pop();
                }
            } else if local == "customXml" {
                level_paths(e, level, path, out);
            }
        }
        path.pop();
    }
}
