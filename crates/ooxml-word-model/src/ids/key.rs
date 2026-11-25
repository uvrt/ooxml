//! Owns: the compact handles block identity is carried by inside a document -- a story's
//! interned index ([`StoryIx`], made by a document's [`StoryTable`]), a block's key
//! ([`BlockKey`], 12 bytes, `Copy`) and a place on or in a block that a layout draws
//! ([`Anchor`], [`Sub`]) -- per the phase 3 identity design.
//!
//! A key is durable where its block has a paraId of its own (a table: its first row's), and
//! otherwise names the block by its place in the version it was made from (an ordinal, a
//! repeat of a paraId, a table's row): such keys live only in versions not yet stamped. The
//! part's index makes and resolves keys ([`super::PartIds::key_of`],
//! [`super::PartIds::entry_of`]); a key is never written or kept across a reopen: ids
//! ([`super::BlockId`]) are.
//!
//! Identity comes from the index, never from an element's address: the tree shares equal
//! subtrees, so two blocks may be one element in memory (ADR-008, docs/architecture §7).
//!
//! Must not: be a memo key (memos are keyed by content), or resolve itself (the index does).

use super::block_id::{BlockKind, ParaId, StoryName};
use std::collections::HashMap;

/// A story's index in its document's [`StoryTable`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct StoryIx(u16);

impl StoryIx {
    /// The main part's story: the first a table interns.
    pub const BODY: StoryIx = StoryIx(0);

    /// Its number.
    #[must_use]
    pub fn get(self) -> u16 {
        self.0
    }
}

/// A document's stories by index: interned once, never removed (an index stays the story's
/// in every version), the body first.
#[derive(Clone, Debug)]
pub struct StoryTable {
    names: Vec<StoryName>,
    by_name: HashMap<StoryName, StoryIx>,
}

impl Default for StoryTable {
    fn default() -> Self {
        StoryTable::new()
    }
}

impl StoryTable {
    /// A table holding the body.
    #[must_use]
    pub fn new() -> StoryTable {
        let body = StoryName::body();
        StoryTable {
            names: vec![body.clone()],
            by_name: HashMap::from([(body, StoryIx::BODY)]),
        }
    }

    /// The index of the story `name`, interned where it is new; `None` once 65,536 stories
    /// are interned.
    pub fn intern(&mut self, name: &str) -> Option<StoryIx> {
        if let Some(ix) = self.get(name) {
            return Some(ix);
        }
        let ix = StoryIx(u16::try_from(self.names.len()).ok()?);
        let story = StoryName::new(name);
        self.names.push(story.clone());
        self.by_name.insert(story, ix);
        Some(ix)
    }

    /// The index of the story `name`, where it is interned.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<StoryIx> {
        self.by_name.get(&StoryName::new(name)).copied()
    }

    /// The story at `ix`.
    #[must_use]
    pub fn name(&self, ix: StoryIx) -> Option<&StoryName> {
        self.names.get(usize::from(ix.0))
    }

    /// How many stories are interned.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Never: the body is always interned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// How a key names its block within its story.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KeyName {
    /// By a paraId of its own (a table: its first row's), unique among the story's blocks of
    /// its kind.
    Durable(ParaId),
    /// The `k`-th repeat (from 1) of a paragraph's paraId.
    Repeat(ParaId, u32),
    /// By its ordinal among the story's paragraphs or tables: a block without a durable id
    /// (no paraId, an invalid or ambiguously written one, a repeat of an invalid one).
    Ordinal(u32),
    /// The `row`-th own row (from 0) of the table with ordinal `table`: a row without a
    /// durable id.
    RowAt {
        /// The table's ordinal among the story's tables.
        table: u32,
        /// The row's place among the table's own rows.
        row: u32,
    },
}

/// A block's compact handle: its story, kind and name, in 12 bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockKey {
    story: StoryIx,
    kind: BlockKind,
    tag: Tag,
    value: u32,
    extra: u32,
}

/// Which [`KeyName`] a key holds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
enum Tag {
    Durable,
    Repeat,
    Ordinal,
    RowAt,
}

const _: () = assert!(std::mem::size_of::<BlockKey>() == 12);

impl BlockKey {
    /// The key of the block of kind `kind` named `name` in the story `story`.
    #[must_use]
    pub fn new(story: StoryIx, kind: BlockKind, name: KeyName) -> BlockKey {
        let (tag, value, extra) = match name {
            KeyName::Durable(id) => (Tag::Durable, id.get(), 0),
            KeyName::Repeat(id, k) => (Tag::Repeat, id.get(), k),
            KeyName::Ordinal(n) => (Tag::Ordinal, n, 0),
            KeyName::RowAt { table, row } => (Tag::RowAt, table, row),
        };
        BlockKey {
            story,
            kind,
            tag,
            value,
            extra,
        }
    }

    /// Its story.
    #[must_use]
    pub fn story(self) -> StoryIx {
        self.story
    }

    /// Its block's kind.
    #[must_use]
    pub fn kind(self) -> BlockKind {
        self.kind
    }

    /// How it names its block.
    #[must_use]
    pub fn name(self) -> KeyName {
        let para = || ParaId::new(self.value);
        match self.tag {
            // made from a `ParaId`, so one again
            Tag::Durable => para().map_or(KeyName::Ordinal(self.value), KeyName::Durable),
            Tag::Repeat => para().map_or(KeyName::Ordinal(self.value), |id| KeyName::Repeat(id, self.extra)),
            Tag::Ordinal => KeyName::Ordinal(self.value),
            Tag::RowAt => KeyName::RowAt {
                table: self.value,
                row: self.extra,
            },
        }
    }

    /// Whether it stays its block's across edits.
    #[must_use]
    pub fn durable(self) -> bool {
        self.tag == Tag::Durable
    }
}

impl std::fmt::Debug for BlockKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockKey")
            .field("story", &self.story.0)
            .field("kind", &self.kind)
            .field("name", &self.name())
            .finish()
    }
}

/// Where on or in its block an anchor is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Sub {
    /// The block itself.
    Whole,
    /// A paragraph's run, by its ordinal among the runs its projection reads.
    Run(u32),
    /// A paragraph's list label.
    Label,
    /// A paragraph's drawing, by its ordinal among the drawings its projection reads.
    Drawing(u32),
    /// A table's cell: its row among the table's rows and its place among the row's cells,
    /// each counted through row- and cell-level wrappers (content controls, custom XML).
    Cell(u16, u16),
    /// The line numbers of the section with this ordinal (`w:sectPr[k]/w:lnNumType`).
    LineNumbers(u32),
    /// The column separators of the section with this ordinal (`w:sectPr[k]/w:cols`).
    Cols(u32),
}

/// A place a layout draws: a block and where on or in it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Anchor {
    /// The block.
    pub block: BlockKey,
    /// Where on or in it.
    pub sub: Sub,
}

impl Anchor {
    /// The block itself.
    #[must_use]
    pub fn whole(block: BlockKey) -> Anchor {
        Anchor { block, sub: Sub::Whole }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_key_gives_back_the_name_it_was_made_with() {
        let id = ParaId::new(0x7FFF_FFFF).unwrap();
        for name in [
            KeyName::Durable(id),
            KeyName::Repeat(id, 3),
            KeyName::Ordinal(0),
            KeyName::Ordinal(u32::MAX),
            KeyName::RowAt { table: 2, row: 9 },
        ] {
            for kind in [BlockKind::Paragraph, BlockKind::Table, BlockKind::Row] {
                let key = BlockKey::new(StoryIx(7), kind, name);
                assert_eq!((key.story(), key.kind(), key.name()), (StoryIx(7), kind, name));
                assert_eq!(key.durable(), matches!(name, KeyName::Durable(_)));
            }
        }
        assert_ne!(
            BlockKey::new(StoryIx::BODY, BlockKind::Paragraph, KeyName::Ordinal(5)),
            BlockKey::new(StoryIx::BODY, BlockKind::Table, KeyName::Ordinal(5))
        );
    }

    #[test]
    fn a_story_keeps_its_index() {
        let mut table = StoryTable::new();
        assert_eq!(table.get("body"), Some(StoryIx::BODY));
        let header = table.intern("header1").unwrap();
        assert_eq!(table.intern("header1"), Some(header));
        assert_eq!(table.intern("body"), Some(StoryIx::BODY));
        assert_eq!(table.name(header).map(StoryName::as_str), Some("header1"));
        assert_eq!(table.len(), 2);
    }
}
