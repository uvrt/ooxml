//! Owns: a version's names ([`VersionNames`]: its story parts' indexes, each found from the
//! version before's where its tree changed, taken as it was where it did not) and its
//! blocks' keys (phase 3 identity design, LK1, LK2c): every paragraph and table of the
//! body, the headers and footers and the notes the model reads keyed from its story's index
//! ([`Keyer`]), a block taken from a memo only where its keys are its own wherever it
//! stands.
//!
//! A key read from the index is the block's own wherever it stands (every paragraph and row
//! has a paraId of its own from the document's open): it is kept with the block it was read
//! with. A table without rows is keyed by its place among the story's tables: a block
//! holding one is read again where it is taken.
//!
//! The reader keys each block as it reads it, by its place ([`KeyOf`]); a block it reads
//! where no index keys it is named by its path.
//!
//! Must not: key a memo (keys name blocks; memos are keyed by content, ADR-008), or name a
//! text box's blocks (no story's index holds them).

use crate::docx::{Kept as DocxKept, StoryPart, VersionNames};
use crate::ids::{BlockKey, StoryIx, StoryTable};
use crate::model::Block;
use crate::read::KeyOf;
use ooxml_xml::Tree;
use std::collections::HashMap;
use std::sync::Arc;

/// One document's story parts as its versions read them: each version's names are made
/// from the version before's.
#[derive(Debug, Default)]
pub(super) struct Names {
    /// Every story the document's versions named, by index.
    stories: StoryTable,
    /// The last version's parts, by part name.
    last: HashMap<String, Arc<StoryPart>>,
    /// This version's parts, by part name, as they are found.
    now: HashMap<String, (StoryIx, Arc<StoryPart>)>,
    /// What a story part keeps beside its ids (unused by naming: never filled).
    kept: Arc<DocxKept>,
}

impl Names {
    /// The part `part` of story `story` in this version, whose tree is `tree`: the last
    /// version's where it is the same tree, else found from it. `None` past the story
    /// table's limit.
    pub(super) fn part(&mut self, part: &str, story: &str, tree: &Tree) -> Option<(StoryIx, Arc<StoryPart>)> {
        if let Some(found) = self.now.get(part) {
            return Some(found.clone());
        }
        let ix = self.stories.intern(story)?;
        let earlier = self.last.get(part).filter(|p| p.story == story);
        let made = match earlier {
            Some(same) if Arc::ptr_eq(same.root(), tree.root()) => Arc::clone(same),
            _ => Arc::new(StoryPart::new(
                part.to_owned(),
                story.to_owned(),
                tree.clone(),
                earlier.map(|e| &**e),
                &self.kept,
            )),
        };
        self.now.insert(part.to_owned(), (ix, Arc::clone(&made)));
        Some((ix, made))
    }

    /// This version's names; the next version's parts are found from its.
    pub(super) fn finish(&mut self) -> Arc<VersionNames> {
        let now = std::mem::take(&mut self.now);
        let names = VersionNames::from_parts(now.values().map(|(ix, part)| (*ix, Arc::clone(part))));
        self.last = now.into_iter().map(|(name, (_, part))| (name, part)).collect();
        Arc::new(names)
    }
}

/// Keys one story's blocks from its part's index, by their places, as the reader reads them.
pub(super) struct Keyer<'a> {
    part: &'a StoryPart,
    story: StoryIx,
}

impl<'a> Keyer<'a> {
    /// The keyer of `part`, the story `story`'s.
    pub(super) fn new(story: StoryIx, part: &'a StoryPart) -> Keyer<'a> {
        Keyer { part, story }
    }

    /// Whether `blocks`, read before (a memo's), are this part's wherever they stand now:
    /// every key in them durable and of this story. A rowless table's key, or a path where
    /// none, names a block by its place, and the blocks are read again (ADR-008: memos are
    /// keyed by content).
    pub(super) fn reusable(&self, blocks: &[Block]) -> bool {
        blocks.iter().all(|b| durable(b, self.story))
    }
}

impl KeyOf for Keyer<'_> {
    fn key_at(&self, place: &[u32]) -> Option<BlockKey> {
        self.part.ids.key_of(self.story, self.part.ids.at(place)?)
    }

    fn path_at(&self, place: &[u32]) -> Option<String> {
        crate::docx::names::block_path(self.part, place)
    }
}

/// Whether every key in `block` is durable and of `story`.
fn durable(block: &Block, story: StoryIx) -> bool {
    let ok = |key: Option<BlockKey>| key.is_some_and(|k| k.durable() && k.story() == story);
    match block {
        Block::Paragraph(p) => ok(p.key),
        Block::Table(t) => {
            ok(t.key)
                && t.rows
                    .iter()
                    .flat_map(|r| r.iter())
                    .flat_map(|c| c.iter())
                    .all(|b| durable(b, story))
        }
    }
}
