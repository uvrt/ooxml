//! Owns: a part's blocks by key ([`BlockKey`]) -- the key of a paragraph, table or row the
//! index holds ([`PartIds::key_of`], [`PartIds::row_key_of`]), the block a key names
//! ([`PartIds::entry_of`], [`PartIds::row_of`]), its place and id -- and a container child's
//! path step (its local name and its count among its namesakes, from the segment tree's
//! prefix counts), each in O(log n) plus the segment it lands in.
//!
//! A paraId names a block durably where the block is its first occurrence (in any case) and
//! the value is written one way only in the part: a value written with fewer than eight
//! digits and also with more (`1A`, `0000001A`) is two ids of one value, and its blocks are
//! keyed by ordinal.
//!
//! Must not: name a block any other way than the index does, or key a block by its
//! element's address (equal subtrees are shared).

use super::segment::{Block, Kind};
use super::{Entry, Located, PartIds, Path};
use crate::ids::block_id::{BlockId, BlockKind, ParaId};
use crate::ids::key::{BlockKey, KeyName, StoryIx};
use crate::ids::valid_long_hex;
use std::sync::Arc;

impl PartIds {
    /// How many paragraphs and rows have a valid paraId written with fewer than eight digits.
    fn short_total(&self) -> usize {
        self.before.short as usize + self.total().short + self.after.short as usize
    }

    /// The one spelling (upper-cased) the value `id` is written in the part, `None` where it
    /// is written in none or in more than one.
    fn spelling(&self, id: ParaId) -> Option<String> {
        let value = id.get();
        if self.short_total() == 0 {
            let spelled = format!("{value:08X}");
            return self.occurrences.get(spelled.as_str()).map(|_| spelled);
        }
        let minimal = format!("{value:X}").len();
        let mut found = None;
        for width in minimal..=8 {
            let spelled = format!("{value:0width$X}");
            if self.occurrences.get(spelled.as_str()).is_some() {
                if found.is_some() {
                    return None;
                }
                found = Some(spelled);
            }
        }
        found
    }

    /// Whether every key the index makes follows from its block's content: every paragraph
    /// and row has a valid paraId of its own, each written with eight digits (so one
    /// spelling per value). A durable key read from such a part is its block's whatever its
    /// place, in every version whose part is such too.
    #[must_use]
    pub fn keys_by_content(&self) -> bool {
        self.fully_stamped() && self.short_total() == 0
    }

    /// `block`'s paraId where it is valid and written one way only in the part.
    fn unique_id(&self, block: &Block) -> Option<ParaId> {
        let raw = block.raw.as_deref()?;
        let id = ParaId::parse(raw)?;
        if self.short_total() == 0 {
            // every valid paraId has eight digits: one spelling per value
            return Some(id);
        }
        (self.spelling(id).as_deref() == block.upper.as_deref()).then_some(id)
    }

    /// The `nth` occurrence (from 0, in any case) of the paraId `id`, written one way only.
    fn occurrence(&self, id: ParaId, nth: usize) -> Option<(Located<'_>, usize)> {
        let upper = self.spelling(id)?;
        let list = self.occurrences.get(upper.as_str())?;
        let label = *list.get(nth)?;
        let earlier = list.partition_point(|l| *l < label);
        let at = self.labelled(label)?;
        let k = at
            .body
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.upper.as_deref() == Some(upper.as_str()))
            .nth(nth - earlier)
            .map(|(k, _)| k)?;
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

    /// Whether the row at place `r` of `at`'s blocks is named by its own paraId (`tr:`):
    /// valid and its first occurrence.
    fn row_named(&self, at: &Located<'_>, r: usize, row: &Block) -> bool {
        row.raw.as_deref().is_some_and(valid_long_hex) && self.repeat_of(at, r, row) == 0
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

    /// The key of the block (paragraph, table or row) at place `k` of `at`'s blocks.
    pub(super) fn key_in(&self, story: StoryIx, at: &Located<'_>, k: usize) -> Option<BlockKey> {
        let block = at.body.blocks.get(k)?;
        let (kind, name) = match block.kind {
            Kind::P => {
                let n = at.paragraphs + block.ordinal as usize;
                let name = match self.unique_id(block) {
                    Some(id) => match self.repeat_of(at, k, block) {
                        0 => KeyName::Durable(id),
                        repeat => {
                            u32::try_from(repeat).map_or(KeyName::Ordinal(ordinal(n)?), |r| KeyName::Repeat(id, r))
                        }
                    },
                    None => KeyName::Ordinal(ordinal(n)?),
                };
                (BlockKind::Paragraph, name)
            }
            Kind::Tbl => {
                let n = at.tables + block.ordinal as usize;
                let first = Self::own_rows(at, k).into_iter().next();
                let id = first
                    .filter(|(r, row)| self.row_named(at, *r, row))
                    .and_then(|(_, row)| self.unique_id(row));
                (
                    BlockKind::Table,
                    id.map_or(KeyName::Ordinal(ordinal(n)?), KeyName::Durable),
                )
            }
            Kind::Tr => {
                let id = self.row_named(at, k, block).then(|| self.unique_id(block)).flatten();
                let name = match id {
                    Some(id) => KeyName::Durable(id),
                    None => {
                        let owner = block.owner? as usize;
                        let table = at.tables + at.body.blocks.get(owner)?.ordinal as usize;
                        let row = Self::own_rows(at, owner).iter().position(|(r, _)| *r == k)?;
                        KeyName::RowAt {
                            table: ordinal(table)?,
                            row: ordinal(row)?,
                        }
                    }
                };
                (BlockKind::Row, name)
            }
        };
        Some(BlockKey::new(story, kind, name))
    }

    /// The paragraph or table `key` names in this part (the part of `key`'s story); `None`
    /// for a row's key, or a key this version's index does not make.
    #[must_use]
    pub fn entry_of(&self, key: BlockKey) -> Option<Entry> {
        let found = match (key.kind(), key.name()) {
            (BlockKind::Paragraph, KeyName::Ordinal(n)) => Entry::Paragraph(n as usize),
            (BlockKind::Paragraph, KeyName::Durable(id)) => self.paragraph_entry(id, 0)?,
            (BlockKind::Paragraph, KeyName::Repeat(id, r)) => self.paragraph_entry(id, r as usize)?,
            (BlockKind::Table, KeyName::Ordinal(n)) => Entry::Table(n as usize),
            (BlockKind::Table, KeyName::Durable(id)) => {
                let (at, k) = self.occurrence(id, 0)?;
                let row = at.body.blocks.get(k).filter(|b| b.kind == Kind::Tr)?;
                let table = at.body.blocks.get(row.owner? as usize)?;
                Entry::Table(at.tables + table.ordinal as usize)
            }
            _ => return None,
        };
        (self.key_of(key.story(), found) == Some(key)).then_some(found)
    }

    fn paragraph_entry(&self, id: ParaId, nth: usize) -> Option<Entry> {
        let (at, k) = self.occurrence(id, nth)?;
        let block = at.body.blocks.get(k).filter(|b| b.kind == Kind::P)?;
        Some(Entry::Paragraph(at.paragraphs + block.ordinal as usize))
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
        let found = match (key.kind(), key.name()) {
            (BlockKind::Row, KeyName::RowAt { table, row }) => (table as usize, row as usize),
            (BlockKind::Row, KeyName::Durable(id)) => {
                let (at, k) = self.occurrence(id, 0)?;
                let row = at.body.blocks.get(k).filter(|b| b.kind == Kind::Tr)?;
                let owner = row.owner? as usize;
                let table = at.body.blocks.get(owner)?;
                let place = Self::own_rows(&at, owner).iter().position(|(r, _)| *r == k)?;
                (at.tables + table.ordinal as usize, place)
            }
            _ => return None,
        };
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
                let (r, block) = *Self::own_rows(&at, k).get(row)?;
                let id = match self.row_raw(&at, (r, block)) {
                    Some(raw) => format!("{}tr:{raw}", self.prefix()),
                    None => format!("{}/r{row}", self.table_head_in(&at, k)?.id),
                };
                Some(BlockId::from(id))
            }
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

/// `n` as a key's ordinal.
fn ordinal(n: usize) -> Option<u32> {
    u32::try_from(n).ok()
}
