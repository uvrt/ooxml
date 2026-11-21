//! Owns: what every paragraph, table and row of one story part is called -- docx-agent's
//! `PartIndex` (`docx_agent/edit/ids.py`): `p:<paraId>` (prefixed with the story outside
//! the body), `p@<story>/<n>` for a paragraph without a paraId, `p:<paraId>#<k>` for the
//! k-th repeat of one, `t:<first row's paraId>` or `t@<story>/<n>` for a table, `tr:` or
//! `<table>/r<k>` for a row.
//!
//! The index is persistent (design section 7): one segment per child of the part's block
//! container (the body, or a story's root), held in a [`MonoidTree`] that counts their
//! paragraphs and tables, and the paraIds' occurrences in a [`PersistentMap`]. A later
//! version's index shares every segment whose child it shares ([`PartIds::after`]), so an
//! edit costs the blocks it changed, not the part. Names are made when asked for: a
//! block's ordinal and repeat number are found in O(log n).
//!
//! Elements are found by their place in the part (a path of child indexes from the root),
//! never by pointer: the tree shares identical subtrees, so two empty paragraphs may be one
//! element in memory.
//!
//! Must not: stamp, generate or remember ids (the ledger does), or resolve an address.

mod build;
mod query;
mod segment;

use crate::ids::valid_long_hex;
use ooxml_derive::{Measured, Monoid, MonoidTree, PersistentMap};
use ooxml_xml::Element;
use segment::{Block, Body, Kind};
use std::sync::Arc;

/// A place in a part: the child indexes from the root (every node counted, as cursors do).
pub type Path = Box<[u32]>;

/// The label of the blocks before the container (outside it, below the root).
const LABEL_BEFORE: u64 = 0;
/// The label of the blocks after the container.
const LABEL_AFTER: u64 = u64::MAX;

/// How a paragraph's id stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdState {
    /// A valid paraId, the first of its value.
    Ok,
    /// No paraId: a positional id, volatile.
    Missing,
    /// A repeat of an earlier paraId (`#k`), volatile.
    Repeat,
    /// A paraId out of range (not `0 < id < 0x80000000`).
    Invalid,
}

impl IdState {
    /// Whether the id can change without the paragraph being edited.
    #[must_use]
    pub fn volatile(self) -> bool {
        matches!(self, IdState::Missing | IdState::Repeat)
    }
}

/// A paragraph's id.
#[derive(Debug, Clone)]
pub struct ParagraphId {
    /// The paragraph.
    pub element: Arc<Element>,
    /// Its place in the part.
    pub path: Path,
    /// Its id.
    pub id: String,
    /// The `w14:paraId` as written.
    pub raw: Option<String>,
    /// How the id stands.
    pub state: IdState,
    /// Its 0-based place among the story's paragraphs (live ones, document order).
    pub index: usize,
}

/// A table's id and its rows'.
#[derive(Debug, Clone)]
pub struct TableId {
    /// The table.
    pub element: Arc<Element>,
    /// Its place in the part.
    pub path: Path,
    /// Its id.
    pub id: String,
    /// Its own rows (not a nested table's), each with its place and id.
    pub rows: Vec<(Path, String)>,
}

/// One entry of the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// The paragraph with this ordinal ([`PartIds::paragraphs`]).
    Paragraph(usize),
    /// The table with this ordinal ([`PartIds::tables`]).
    Table(usize),
}

/// A segment in the tree: its order key and its blocks.
#[derive(Debug, Clone)]
struct Seg {
    label: u64,
    body: Arc<Body>,
}

/// What a stretch of segments holds.
#[derive(Debug, Clone, Copy, Default)]
struct Count {
    paragraphs: usize,
    tables: usize,
    unstamped: usize,
    /// The largest label (labels grow in document order).
    last: u64,
}

impl Monoid for Count {
    fn empty() -> Self {
        Count::default()
    }
    fn combine(&self, other: &Self) -> Self {
        Count {
            paragraphs: self.paragraphs + other.paragraphs,
            tables: self.tables + other.tables,
            unstamped: self.unstamped + other.unstamped,
            last: self.last.max(other.last),
        }
    }
}

impl Measured for Seg {
    type Summary = Count;
    fn measure(&self) -> Count {
        Count::of(self)
    }
}

/// Every paragraph and table of one story part, by id and by place.
#[derive(Debug, Clone)]
pub struct PartIds {
    /// The story's name (`body`, `header1`, `footnotes`...).
    story: Arc<str>,
    /// The place of the block container (the body: one index; a story root: none).
    container: Arc<[u32]>,
    /// Blocks below the root before the container, and after it (places from the root).
    before: Arc<Body>,
    after: Arc<Body>,
    /// One segment per container child.
    segments: MonoidTree<Seg>,
    /// Each upper-cased paraId's occurrences, by label, in document order.
    occurrences: PersistentMap<Arc<str>, Arc<[u64]>>,
    /// How many paraIds occur more than once.
    repeated: usize,
}

/// Where a segment is: its blocks, the place its `rel`s are below, its label, and the
/// paragraphs and tables before it.
struct Located<'a> {
    body: &'a Body,
    base: Vec<u32>,
    label: u64,
    paragraphs: usize,
    tables: usize,
}

impl PartIds {
    /// The story's name.
    #[must_use]
    pub fn story(&self) -> &str {
        &self.story
    }

    fn prefix(&self) -> String {
        if &*self.story == "body" {
            String::new()
        } else {
            format!("{}/", self.story)
        }
    }

    fn total(&self) -> Count {
        self.segments.summary()
    }

    /// The number of paragraphs.
    #[must_use]
    pub fn paragraph_count(&self) -> usize {
        self.before.paragraphs.len() + self.total().paragraphs + self.after.paragraphs.len()
    }

    /// The number of tables (nested ones included).
    #[must_use]
    pub fn table_count(&self) -> usize {
        self.before.tables.len() + self.total().tables + self.after.tables.len()
    }

    /// The place of the story's block container: the body's (one index), or none for a
    /// story whose root holds its blocks.
    #[must_use]
    pub fn container(&self) -> &[u32] {
        &self.container
    }

    /// Whether every paragraph and row has a valid paraId of its own (nothing to stamp).
    #[must_use]
    pub fn fully_stamped(&self) -> bool {
        self.repeated == 0 && self.total().unstamped == 0 && self.before.unstamped == 0 && self.after.unstamped == 0
    }

    fn outer_before(&self) -> Located<'_> {
        Located {
            body: &self.before,
            base: Vec::new(),
            label: LABEL_BEFORE,
            paragraphs: 0,
            tables: 0,
        }
    }

    fn outer_after(&self) -> Located<'_> {
        let total = self.total();
        Located {
            body: &self.after,
            base: Vec::new(),
            label: LABEL_AFTER,
            paragraphs: self.before.paragraphs.len() + total.paragraphs,
            tables: self.before.tables.len() + total.tables,
        }
    }

    /// The segment at container child `k`, with what is before it.
    fn segment(&self, k: usize, before: &Count) -> Option<Located<'_>> {
        let seg = self.segments.get(k)?;
        let mut base = self.container.to_vec();
        base.push(u32::try_from(k).ok()?);
        Some(Located {
            body: &seg.body,
            base,
            label: seg.label,
            paragraphs: self.before.paragraphs.len() + before.paragraphs,
            tables: self.before.tables.len() + before.tables,
        })
    }

    /// The segment holding the paragraph (`tables`: table) with ordinal `n`.
    fn holding(&self, n: usize, tables: bool) -> Option<Located<'_>> {
        let own = |b: &Body| if tables { b.tables.len() } else { b.paragraphs.len() };
        let first = own(&self.before);
        if n < first {
            return Some(self.outer_before());
        }
        let total = self.total();
        let inner = if tables { total.tables } else { total.paragraphs };
        if n - first >= inner {
            return (n - first - inner < own(&self.after)).then(|| self.outer_after());
        }
        let wanted = n - first;
        let (k, before) = self.segments.seek(|c| {
            let had = if tables { c.tables } else { c.paragraphs };
            had > wanted
        })?;
        self.segment(k, &before)
    }

    /// The segment whose label is `label`.
    fn labelled(&self, label: u64) -> Option<Located<'_>> {
        match label {
            LABEL_BEFORE => Some(self.outer_before()),
            LABEL_AFTER => Some(self.outer_after()),
            _ => {
                let (k, before) = self.segments.seek(|c| c.last >= label)?;
                let located = self.segment(k, &before)?;
                (located.label == label).then_some(located)
            }
        }
    }

    /// How many times `block`'s paraId (in any case) was written before it.
    fn repeat_of(&self, at: &Located<'_>, k: usize, block: &Block) -> usize {
        let Some(upper) = block.upper.as_ref().filter(|_| self.repeated > 0) else {
            return 0;
        };
        let Some(list) = self.occurrences.get(upper) else {
            return 0;
        };
        if list.len() < 2 {
            return 0;
        }
        let earlier = list.partition_point(|l| *l < at.label);
        let here = at
            .body
            .blocks
            .iter()
            .take(k)
            .filter(|b| b.upper.as_ref() == Some(upper))
            .count();
        earlier + here
    }

    fn path_of(at: &Located<'_>, block: &Block) -> Path {
        let mut path = at.base.clone();
        path.extend_from_slice(&block.rel);
        path.into()
    }

    /// The paragraph at place `k` of `at`'s blocks.
    fn paragraph_in(&self, at: &Located<'_>, k: usize) -> Option<ParagraphId> {
        let block = at.body.blocks.get(k).filter(|b| b.kind == Kind::P)?;
        let index = at.paragraphs + block.ordinal as usize;
        let repeat = self.repeat_of(at, k, block);
        let prefix = self.prefix();
        let (id, state) = match block.raw.as_deref() {
            None => (format!("p@{}/{index}", self.story), IdState::Missing),
            Some(raw) if repeat > 0 => (format!("{prefix}p:{raw}#{repeat}"), IdState::Repeat),
            Some(raw) if valid_long_hex(raw) => (format!("{prefix}p:{raw}"), IdState::Ok),
            Some(raw) => (format!("{prefix}p:{raw}"), IdState::Invalid),
        };
        Some(ParagraphId {
            element: Arc::clone(&block.element),
            path: Self::path_of(at, block),
            id,
            raw: block.raw.as_deref().map(str::to_owned),
            state,
            index,
        })
    }

    /// The table at place `k` of `at`'s blocks.
    fn table_in(&self, at: &Located<'_>, k: usize) -> Option<TableId> {
        let block = at.body.blocks.get(k).filter(|b| b.kind == Kind::Tbl)?;
        let number = at.tables + block.ordinal as usize;
        let prefix = self.prefix();
        let owner = u32::try_from(k).ok();
        let rows: Vec<(usize, &Block)> = at
            .body
            .blocks
            .iter()
            .enumerate()
            .skip(k + 1)
            .filter(|(_, b)| b.kind == Kind::Tr && b.owner == owner)
            .collect();
        let good = |(r, row): &(usize, &Block)| {
            row.raw
                .as_deref()
                .filter(|raw| valid_long_hex(raw) && self.repeat_of(at, *r, row) == 0)
                .map(str::to_owned)
        };
        let id = match rows.first().and_then(good) {
            Some(first) => format!("{prefix}t:{first}"),
            None => format!("t@{}/{number}", self.story),
        };
        let rows = rows
            .iter()
            .enumerate()
            .map(|(n, found)| {
                let row = match good(found) {
                    Some(raw) => format!("{prefix}tr:{raw}"),
                    None => format!("{id}/r{n}"),
                };
                (Self::path_of(at, found.1), row)
            })
            .collect();
        Some(TableId {
            element: Arc::clone(&block.element),
            path: Self::path_of(at, block),
            id,
            rows,
        })
    }

    /// Every paragraph element, in document order, without naming it (cheaper than
    /// [`PartIds::paragraphs`] where the ids are not wanted).
    pub fn paragraph_elements(&self) -> impl Iterator<Item = &Arc<Element>> + '_ {
        std::iter::once(&*self.before)
            .chain(self.segments.iter().map(|s| &*s.body))
            .chain(std::iter::once(&*self.after))
            .flat_map(|body| {
                body.paragraphs
                    .iter()
                    .filter_map(|k| body.blocks.get(*k as usize))
                    .map(|b| &b.element)
            })
    }

    /// The paragraphs, in document order.
    #[must_use]
    pub fn paragraphs(&self) -> Paragraphs<'_> {
        Paragraphs { ids: self }
    }

    /// The tables, in document order (nested ones included).
    #[must_use]
    pub fn tables(&self) -> Tables<'_> {
        Tables { ids: self }
    }

    /// The segments in document order: the blocks before the container, each container
    /// child's, the blocks after.
    fn located(&self) -> impl Iterator<Item = Located<'_>> + '_ {
        let mut seen = Count::default();
        let inner = self.segments.iter().enumerate().filter_map(move |(k, seg)| {
            let at = self.segment_after(k, seg, &seen);
            seen = seen.combine(&Count::of(seg));
            at
        });
        std::iter::once(self.outer_before())
            .chain(inner)
            .chain(std::iter::once(self.outer_after()))
    }

    fn segment_after<'a>(&'a self, k: usize, seg: &'a Seg, before: &Count) -> Option<Located<'a>> {
        let mut base = self.container.to_vec();
        base.push(u32::try_from(k).ok()?);
        Some(Located {
            body: &seg.body,
            base,
            label: seg.label,
            paragraphs: self.before.paragraphs.len() + before.paragraphs,
            tables: self.before.tables.len() + before.tables,
        })
    }

    /// The entry of the block at `path`, with the segment and place it is at.
    fn find_path(&self, path: &[u32]) -> Option<(Located<'_>, usize)> {
        let c = self.container.len();
        let inside = path.len() > c && path.starts_with(&self.container);
        if inside {
            let k = *path.get(c)? as usize;
            let before = self.segments.prefix(k).ok()?;
            let at = self.segment(k, &before)?;
            let (place, _) = at.body.at(path.get(c + 1..)?)?;
            return Some((at, place));
        }
        for at in [self.outer_before(), self.outer_after()] {
            if let Some((place, _)) = at.body.at(path) {
                return Some((at, place));
            }
        }
        None
    }

    /// The entry an id names, as written (no alias followed).
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Entry> {
        let positional = |kind: &str| -> Option<usize> {
            id.strip_prefix(kind)?
                .strip_prefix(&*self.story)?
                .strip_prefix('/')?
                .parse()
                .ok()
        };
        if let Some(n) = positional("p@") {
            let p = self.paragraphs().get(n)?;
            return (p.id == id).then_some(Entry::Paragraph(n));
        }
        if let Some(n) = positional("t@") {
            let t = self.tables().get(n)?;
            return (t.id == id).then_some(Entry::Table(n));
        }
        let rest = id.strip_prefix(&self.prefix())?;
        if let Some(raw) = rest.strip_prefix("p:") {
            let whole = self.by_raw(raw, 0, Kind::P);
            let repeat = raw
                .rsplit_once('#')
                .and_then(|(raw, k)| Some((raw, k.parse::<usize>().ok().filter(|k| *k > 0)?)));
            let found = whole.or_else(|| repeat.and_then(|(raw, k)| self.by_raw(raw, k, Kind::P)))?;
            let p = self.paragraph_in(&found.0, found.1)?;
            return (p.id == id).then_some(Entry::Paragraph(p.index));
        }
        let raw = rest.strip_prefix("t:")?;
        let (at, k) = self.by_raw(raw, 0, Kind::Tr)?;
        let owner = at.body.blocks.get(k)?.owner? as usize;
        let table = self.table_in(&at, owner)?;
        let number = at.tables + at.body.blocks.get(owner)?.ordinal as usize;
        (table.id == id).then_some(Entry::Table(number))
    }

    /// The `repeat`-th occurrence of the paraId `raw` (in any case), when it is a block of
    /// `kind` written `raw`.
    fn by_raw(&self, raw: &str, repeat: usize, kind: Kind) -> Option<(Located<'_>, usize)> {
        let upper = raw.to_uppercase();
        let list = self.occurrences.get(upper.as_str())?;
        let label = *list.get(repeat)?;
        let earlier = list.partition_point(|l| *l < label);
        let at = self.labelled(label)?;
        let k = at
            .body
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.upper.as_deref() == Some(upper.as_str()))
            .nth(repeat - earlier)
            .map(|(k, _)| k)?;
        let block = at.body.blocks.get(k)?;
        (block.kind == kind && block.raw.as_deref() == Some(raw)).then_some((at, k))
    }

    /// The entry of the paragraph or table at `path`.
    #[must_use]
    pub fn at(&self, path: &[u32]) -> Option<Entry> {
        let (at, k) = self.find_path(path)?;
        let block = at.body.blocks.get(k)?;
        match block.kind {
            Kind::P => Some(Entry::Paragraph(at.paragraphs + block.ordinal as usize)),
            Kind::Tbl => Some(Entry::Table(at.tables + block.ordinal as usize)),
            Kind::Tr => None,
        }
    }

    /// The paragraph at `path`.
    #[must_use]
    pub fn paragraph_at(&self, path: &[u32]) -> Option<ParagraphId> {
        let (at, k) = self.find_path(path)?;
        self.paragraph_in(&at, k)
    }

    /// The table at `path`.
    #[must_use]
    pub fn table_at(&self, path: &[u32]) -> Option<TableId> {
        let (at, k) = self.find_path(path)?;
        self.table_in(&at, k)
    }

    /// The id of the entry `entry`.
    #[must_use]
    pub fn id_of(&self, entry: Entry) -> Option<String> {
        match entry {
            Entry::Paragraph(k) => self.paragraphs().get(k).map(|p| p.id),
            Entry::Table(k) => self.tables().get(k).map(|t| t.id),
        }
    }
}

/// A part's paragraphs, named when read.
#[derive(Clone, Copy)]
pub struct Paragraphs<'a> {
    ids: &'a PartIds,
}

impl<'a> Paragraphs<'a> {
    /// How many there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.paragraph_count()
    }

    /// None.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The first paragraph.
    #[must_use]
    pub fn first(&self) -> Option<ParagraphId> {
        self.get(0)
    }

    /// The last paragraph.
    #[must_use]
    pub fn last(&self) -> Option<ParagraphId> {
        self.get(self.len().checked_sub(1)?)
    }

    /// The paragraph with ordinal `n`, in O(log n).
    #[must_use]
    pub fn get(&self, n: usize) -> Option<ParagraphId> {
        let at = self.ids.holding(n, false)?;
        let local = n.checked_sub(at.paragraphs)?;
        let k = *at.body.paragraphs.get(local)? as usize;
        self.ids.paragraph_in(&at, k)
    }

    /// Every paragraph's element, in document order: what [`Paragraphs::iter`] gives,
    /// without making each one's id.
    pub fn elements(&self) -> impl Iterator<Item = &'a Arc<Element>> + 'a {
        self.ids.located().flat_map(|at| {
            let body = at.body;
            body.paragraphs.iter().filter_map(move |k| {
                body.blocks
                    .get(*k as usize)
                    .filter(|b| b.kind == Kind::P)
                    .map(|b| &b.element)
            })
        })
    }

    /// Every paragraph, in document order.
    pub fn iter(&self) -> impl Iterator<Item = ParagraphId> + 'a {
        let ids = self.ids;
        ids.located().flat_map(move |at| {
            let places: Vec<usize> = at.body.paragraphs.iter().map(|k| *k as usize).collect();
            places.into_iter().filter_map(move |k| ids.paragraph_in(&at, k))
        })
    }
}

/// A part's tables, named when read.
#[derive(Clone, Copy)]
pub struct Tables<'a> {
    ids: &'a PartIds,
}

impl<'a> Tables<'a> {
    /// How many there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.table_count()
    }

    /// None.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The table with ordinal `n`, in O(log n).
    #[must_use]
    pub fn get(&self, n: usize) -> Option<TableId> {
        let at = self.ids.holding(n, true)?;
        let local = n.checked_sub(at.tables)?;
        let k = *at.body.tables.get(local)? as usize;
        self.ids.table_in(&at, k)
    }

    /// Every table, in document order.
    pub fn iter(&self) -> impl Iterator<Item = TableId> + 'a {
        let ids = self.ids;
        ids.located().flat_map(move |at| {
            let places: Vec<usize> = at.body.tables.iter().map(|k| *k as usize).collect();
            places.into_iter().filter_map(move |k| ids.table_in(&at, k))
        })
    }
}
