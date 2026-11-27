//! Owns: what one version of a document changed against another, by block identity
//! ([`VersionDelta`]): the parts other than stories whose content differs ([`PartsDelta`]),
//! and per story either the whole part ([`StoryDelta::Whole`]) or its blocks by key
//! ([`BlockDelta`]: changed, came, gone, moved, renamed, changed only by stamping) -- the
//! phase 3 identity design's change set (section 2), made from the two versions' indexes
//! ([`super::PartIds::delta`]) rather than by each command, so an edit, an undo, a restore
//! and a span of many versions get it from one function.
//!
//! Identity comes from the index, never from an element's address: the tree shares equal
//! subtrees, so a pointer only says that content is the same (ADR-008).
//!
//! Must not: say who made the change or why (the store and the commands do), or name a
//! block by text (the index does, at the edge).

use super::key::{BlockKey, StoryIx};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// One document's identity in this process, whatever its versions: what tells one
/// document's store numbers from another's ([`VersionRef`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocumentId(u64);

impl DocumentId {
    /// A document's identity no other document in this process has.
    #[must_use]
    pub fn fresh() -> DocumentId {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        DocumentId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// One version of one document: the document, and the number its store gave the version
/// (`Seq`, unique within the document's store only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VersionRef {
    /// The document.
    pub document: DocumentId,
    /// The version's number in the document's store.
    pub seq: u64,
}

/// The two versions a [`VersionDelta`] is between: its base, the version it was worked out
/// against, and the version it leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionSpan {
    /// The version it was worked out against.
    pub base: VersionRef,
    /// The version it leads to.
    pub version: VersionRef,
}

/// A version's changes against an earlier version of the same document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionDelta {
    /// The two versions it is between, where the store that holds them names them: what a
    /// layout engine checks before taking the delta for the change from the version it laid
    /// out last ([`VersionSpan`]).
    pub span: Option<VersionSpan>,
    /// The parts that are not stories (styles, numbering, settings, relationships, media...)
    /// whose content differs.
    pub parts: PartsDelta,
    /// The stories whose part differs, in the order of the later version's parts (a story
    /// only the earlier version has last).
    pub stories: Vec<StoryChange>,
}

impl VersionDelta {
    /// The same delta, between the versions `span` names.
    #[must_use]
    pub fn spanning(mut self, span: VersionSpan) -> VersionDelta {
        self.span = Some(span);
        self
    }

    /// The version it was worked out against, where it is named.
    #[must_use]
    pub fn base(&self) -> Option<VersionRef> {
        self.span.map(|s| s.base)
    }

    /// Whether nothing differs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts.changed.is_empty() && self.stories.is_empty()
    }

    /// The change of the story `story`, where its part differs.
    #[must_use]
    pub fn story(&self, story: StoryIx) -> Option<&StoryChange> {
        self.stories.iter().find(|s| s.story == story)
    }

    /// Every part whose content differs (stories' parts and the others), sorted by name.
    #[must_use]
    pub fn changed_parts(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self.parts.changed.iter().map(String::as_str).collect();
        out.extend(self.stories.iter().map(|s| s.part.as_str()));
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The parts that are not stories whose content differs: put, removed or replaced (by tree
/// identity: a part whose tree or bytes the later version shares is the same).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartsDelta {
    /// Their names, sorted.
    pub changed: Vec<String>,
}

/// One story whose part differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryChange {
    /// The story.
    pub story: StoryIx,
    /// Its part's name.
    pub part: String,
    /// What differs in it.
    pub delta: StoryDelta,
}

/// What differs in one story's part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoryDelta {
    /// The part was put or removed, or its block container moved: every block may differ.
    Whole,
    /// Its blocks, by key.
    Blocks(BlockDelta),
}

/// A story's blocks (paragraphs, tables and rows, nested ones included) that differ, by key.
/// Every list is in document order: the later version's, the earlier version's for `gone`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockDelta {
    /// Blocks both versions hold under one key whose content differs beyond the ids stamping
    /// writes (a renamed block whose content differs is here under its new key too).
    pub changed: Vec<BlockKey>,
    /// Blocks only the later version holds, each with the block before it there (`None`: the
    /// story's first).
    pub came: Vec<(BlockKey, Option<BlockKey>)>,
    /// Blocks only the earlier version holds, each with the block before it there that the
    /// later version still holds, by its key there (`None`: none is).
    pub gone: Vec<(BlockKey, Option<BlockKey>)>,
    /// Blocks both versions hold whose order among the others changed.
    pub moved: Vec<Moved>,
    /// Blocks named otherwise in the later version: old key, new key -- a table whose first
    /// row changed, a block stamped with a paraId of its own.
    pub renamed: Vec<(BlockKey, BlockKey)>,
    /// Blocks whose only difference is the ids stamping writes (`w14:paraId`, `w14:textId`,
    /// `wp14:anchorId`, `wp14:editId`), by their key in the later version.
    pub stamp_only: Vec<BlockKey>,
    /// Blocks that did not change themselves whose wrapping in the story's block container
    /// differs (a content control or custom XML put, removed or changed around them), named
    /// by the first block inside.
    pub wrapped: Vec<BlockKey>,
    /// Whether markup of the block container's that holds no block differs (a section's
    /// properties, a bookmark between blocks, an empty content control).
    pub outside: bool,
}

impl BlockDelta {
    /// Each renamed block's earlier key, by its later one (the first rename to it), to look
    /// up per block (a first edit renames every block of the document).
    #[must_use]
    pub fn renamed_from(&self) -> HashMap<BlockKey, BlockKey> {
        let mut from = HashMap::with_capacity(self.renamed.len());
        for (old, new) in &self.renamed {
            from.entry(*new).or_insert(*old);
        }
        from
    }

    /// Whether no block differs and nothing between them does.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty()
            && self.came.is_empty()
            && self.gone.is_empty()
            && self.moved.is_empty()
            && self.renamed.is_empty()
            && self.stamp_only.is_empty()
            && self.wrapped.is_empty()
            && !self.outside
    }
}

/// A block whose order among the others changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moved {
    /// Its key (in the later version: renamed blocks are under their new key).
    pub key: BlockKey,
    /// The block before it in the later version (`None`: the story's first).
    pub after: Option<BlockKey>,
    /// The block before it in the earlier version that the later version still holds, by
    /// its key there (`None`: none is).
    pub was_after: Option<BlockKey>,
}
