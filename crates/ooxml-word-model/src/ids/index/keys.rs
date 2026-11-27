//! Owns: a part's blocks by key ([`BlockKey`]) -- the key of a paragraph, table or row the
//! index holds ([`PartIds::key_of`], [`PartIds::row_key_of`]), the block a key names
//! ([`PartIds::entry_of`], [`PartIds::row_of`]), its place and id -- and a container child's
//! path step (its local name and its count among its namesakes, from the segment tree's
//! prefix counts), each in O(log n) plus the segment it lands in.
//!
//! A paraId names a block durably where the block is the first occurrence of its value
//! (whatever its case or spelling). A block without one (a part not yet stamped) has no key;
//! a table without rows is keyed by its ordinal.
//!
//! Must not: name a block any other way than the index does, or key a block by its
//! element's address (equal subtrees are shared).

use super::segment::{Block, Kind};
use super::{Entry, Located, PartIds, Path};
use crate::ids::block_id::{BlockId, BlockKind, ParaId};
use crate::ids::key::{BlockKey, KeyName, StoryIx};
use std::sync::Arc;

impl PartIds {
    /// `block`'s paraId where it is valid and the first of its value.
    fn unique_id(&self, at: &Located<'_>, k: usize, block: &Block) -> Option<ParaId> {
        let id = ParaId::parse(block.raw.as_deref()?)?;
        (self.repeat_of(at, k, block) == 0).then_some(id)
    }

    /// The first occurrence of the paraId `id`'s value.
    fn occurrence(&self, id: ParaId) -> Option<(Located<'_>, usize)> {
        let upper = format!("{:08X}", id.get());
        let label = *self.occurrences.get(upper.as_str())?.first()?;
        let at = self.labelled(label)?;
        let k = at
            .body
            .blocks
            .iter()
            .position(|b| b.upper.as_deref() == Some(upper.as_str()))?;
        Some((at, k))
    }

    /// The paragraph with ordinal `n`: its segment and place there.
    fn paragraph_place(&self, n: usize) -> Option<(Located<'_>, usize)> {
        let at = self.holding(n, false)?;
        let k = *at.body.paragraphs.get(n.checked_sub(at.paragraphs)?)? as usize;
        Some((at, k))
    }

    /// The table with ordinal `n`: its segment and place there.
    fn table_place(&self, n: usize) -> Option<(Located<'_>, usize)> {
        let at = self.holding(n, true)?;
        let k = *at.body.tables.get(n.checked_sub(at.tables)?)? as usize;
        Some((at, k))
    }

    /// The key of the paragraph or table `entry`, in the story `story` (this part's).
    #[must_use]
    pub fn key_of(&self, story: StoryIx, entry: Entry) -> Option<BlockKey> {
        let (at, k) = match entry {
            Entry::Paragraph(n) => self.paragraph_place(n)?,
            Entry::Table(n) => self.table_place(n)?,
        };
        let kind = at.body.blocks.get(k)?.kind;
        let wanted = match entry {
            Entry::Paragraph(_) => Kind::P,
            Entry::Table(_) => Kind::Tbl,
        };
        (kind == wanted).then(|| self.key_in(story, &at, k)).flatten()
    }

    /// The key of the block (paragraph, table or row) at place `k` of `at`'s blocks; `None`
    /// for a paragraph or row without a paraId of its own, or a table whose first row has
    /// none.
    pub(super) fn key_in(&self, story: StoryIx, at: &Located<'_>, k: usize) -> Option<BlockKey> {
        let block = at.body.blocks.get(k)?;
        let (kind, name) = match block.kind {
            Kind::P => (BlockKind::Paragraph, KeyName::Durable(self.unique_id(at, k, block)?)),
            Kind::Tbl => {
                let name = match Self::own_rows(at, k).first() {
                    Some((r, row)) => KeyName::Durable(self.unique_id(at, *r, row)?),
                    None => KeyName::Rowless(u32::try_from(at.tables + block.ordinal as usize).ok()?),
                };
                (BlockKind::Table, name)
            }
            Kind::Tr => (BlockKind::Row, KeyName::Durable(self.unique_id(at, k, block)?)),
        };
        Some(BlockKey::new(story, kind, name))
    }

    /// The paragraph or table `key` names in this part (the part of `key`'s story); `None`
    /// for a row's key, or a key this version's index does not make.
    #[must_use]
    pub fn entry_of(&self, key: BlockKey) -> Option<Entry> {
        let found = match (key.kind(), key.name()) {
            (BlockKind::Paragraph, KeyName::Durable(id)) => {
                let (at, k) = self.occurrence(id)?;
                let block = at.body.blocks.get(k).filter(|b| b.kind == Kind::P)?;
                Entry::Paragraph(at.paragraphs + block.ordinal as usize)
            }
            (BlockKind::Table, KeyName::Rowless(n)) => Entry::Table(n as usize),
            (BlockKind::Table, KeyName::Durable(id)) => {
                let (at, k) = self.occurrence(id)?;
                let row = at.body.blocks.get(k).filter(|b| b.kind == Kind::Tr)?;
                let table = at.body.blocks.get(row.owner? as usize)?;
                Entry::Table(at.tables + table.ordinal as usize)
            }
            _ => return None,
        };
        (self.key_of(key.story(), found) == Some(key)).then_some(found)
    }

    /// The key of the `row`-th own row (from 0) of the table with ordinal `table`, in the
    /// story `story` (this part's).
    #[must_use]
    pub fn row_key_of(&self, story: StoryIx, table: usize, row: usize) -> Option<BlockKey> {
        let (at, k) = self.table_place(table)?;
        let (r, _) = *Self::own_rows(&at, k).get(row)?;
        self.key_in(story, &at, r)
    }

    /// The table (its ordinal) and own row (its place, from 0) a row's `key` names; `None`
    /// for another key, or one this version's index does not make.
    #[must_use]
    pub fn row_of(&self, key: BlockKey) -> Option<(usize, usize)> {
        let (BlockKind::Row, KeyName::Durable(id)) = (key.kind(), key.name()) else {
            return None;
        };
        let (at, k) = self.occurrence(id)?;
        let row = at.body.blocks.get(k).filter(|b| b.kind == Kind::Tr)?;
        let owner = row.owner? as usize;
        let table = at.body.blocks.get(owner)?;
        let place = Self::own_rows(&at, owner).iter().position(|(r, _)| *r == k)?;
        let found = (at.tables + table.ordinal as usize, place);
        (self.row_key_of(key.story(), found.0, found.1) == Some(key)).then_some(found)
    }

    /// The block `key` names: its segment and place there.
    fn block_of(&self, key: BlockKey) -> Option<(Located<'_>, usize)> {
        match key.kind() {
            BlockKind::Paragraph | BlockKind::Table => match self.entry_of(key)? {
                Entry::Paragraph(n) => self.paragraph_place(n),
                Entry::Table(n) => self.table_place(n),
            },
            BlockKind::Row => {
                let (table, row) = self.row_of(key)?;
                let (at, k) = self.table_place(table)?;
                let (r, _) = *Self::own_rows(&at, k).get(row)?;
                Some((at, r))
            }
        }
    }

    /// The place (child indexes from the root) of the block `key` names.
    #[must_use]
    pub fn place_of(&self, key: BlockKey) -> Option<Path> {
        let (at, k) = self.block_of(key)?;
        Some(Self::path_of(&at, at.body.blocks.get(k)?))
    }

    /// The `w14:paraId` of the paragraph or row `key` names, as written (`None` for a table,
    /// or a block written without one).
    #[must_use]
    pub fn raw_id_of(&self, key: BlockKey) -> Option<Arc<str>> {
        let (at, k) = self.block_of(key)?;
        at.body.blocks.get(k)?.raw.clone()
    }

    /// The id of the block `key` names, as the index names it.
    #[must_use]
    pub fn id_of_key(&self, key: BlockKey) -> Option<BlockId> {
        match key.kind() {
            BlockKind::Paragraph | BlockKind::Table => self.id_of(self.entry_of(key)?).map(BlockId::from),
            BlockKind::Row => {
                let (table, row) = self.row_of(key)?;
                let (at, k) = self.table_place(table)?;
                let (_, block) = *Self::own_rows(&at, k).get(row)?;
                Some(BlockId::from(format!("{}tr:{}", self.prefix(), block.raw.as_deref()?)))
            }
        }
    }

    /// The key of the paragraph, table or row at `place` (child indexes from the root), in
    /// the story `story` (this part's); `None` where no block the index holds is there.
    #[must_use]
    pub fn key_at(&self, story: StoryIx, place: &[u32]) -> Option<BlockKey> {
        let (at, k) = self.find_path(place)?;
        self.key_in(story, &at, k)
    }

    /// The child index of the block container's `n`-th child (from 1) of local name `local`
    /// (an element, whatever its namespace): the inverse of [`PartIds::container_step`].
    /// O(log n) for the names the segment tree counts (`TOP_NAMES`), a walk of the children
    /// for any other.
    #[must_use]
    pub fn container_child(&self, local: &str, n: usize) -> Option<usize> {
        let wanted = u32::try_from(n.checked_sub(1)?).ok()?;
        match super::TOP_NAMES.iter().position(|name| *name == local) {
            Some(slot) => {
                let (k, _) = self
                    .segments
                    .seek(|c| c.names.get(slot).is_some_and(|count| *count > wanted))?;
                Some(k)
            }
            None => self
                .segments
                .iter()
                .enumerate()
                .filter(|(_, s)| s.body.local() == Some(local))
                .nth(wanted as usize)
                .map(|(k, _)| k),
        }
    }

    /// The local name of the block container's child `k` (an element, whatever its
    /// namespace) and its count among the container's children of that local name, from 1:
    /// its step in a path from the container. O(log n) for the names the segment tree
    /// counts (`TOP_NAMES`), a walk of the children before it for any other.
    #[must_use]
    pub fn container_step(&self, k: usize) -> Option<(&str, u32)> {
        let seg = self.segments.get(k)?;
        let local = seg.body.local()?;
        let before = match seg.body.top_name() {
            Some(slot) => *self.segments.prefix(k).ok()?.names.get(slot)?,
            None => {
                let same = self.segments.iter().take(k).filter(|s| s.body.local() == Some(local));
                u32::try_from(same.count()).ok()?
            }
        };
        Some((local, before + 1))
    }
}
