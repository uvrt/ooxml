//! Owns: a story's change between two versions by block identity ([`PartIds::delta`]): the
//! container children the later version does not share with the earlier one are found by
//! identity (`ooxml_xml::changed_runs`), and only their blocks are keyed and compared -- by
//! key, then by content (the same allocation, equal, or equal but for the ids stamping
//! writes) -- the rest of the story being the same blocks under the same keys: every block
//! has a paraId of its own from the document's open, so a key follows from content.
//!
//! Renames pair blocks both versions hold under other keys: a table with the table sharing
//! one of its rows (its first row changed), and a gone block with a came one of the same
//! content but stamps, aligned in order (a copy under new ids replaced it). The same
//! alignment pairs a part another writer put without ids with the version before's
//! ([`PartIds::aligned`]).
//!
//! Must not: key identity by an element's address (a pointer only says content is the same,
//! ADR-008), or name a block by text.

use super::build::{container, container_children};
use super::segment::Kind;
use super::{Count, Located, PartIds, Path};
use crate::ids::delta::{BlockDelta, Moved, StoryDelta};
use crate::ids::key::{BlockKey, StoryIx};
use crate::ids::{W, W14, WP14};
use ooxml_base::difflib::{opcodes, Tag};
use ooxml_derive::Monoid;
use ooxml_xml::{Children, Element, Node};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

/// One block compared: its key, element, kind, the stretch it is in (`run`: blocks before the
/// container 0, the container's changed runs from 1, blocks after it last) and its container
/// child (`None` outside the container), and the item of the table holding it (a row's).
struct Item<'a> {
    key: BlockKey,
    element: &'a Arc<Element>,
    kind: Kind,
    run: usize,
    seg: Option<usize>,
    owner: Option<usize>,
}

/// One side's blocks compared, in document order, by key and by container child, with the
/// block before each stretch (`None`: none).
struct Side<'a> {
    items: Vec<Item<'a>>,
    by_key: HashMap<BlockKey, usize>,
    /// Per container child compared, the items of its blocks.
    by_seg: HashMap<usize, Range<usize>>,
    /// Per stretch, the key of the block just before it.
    before_run: HashMap<usize, Option<BlockKey>>,
}

impl<'a> Side<'a> {
    fn new() -> Self {
        Side {
            items: Vec::new(),
            by_key: HashMap::new(),
            by_seg: HashMap::new(),
            before_run: HashMap::new(),
        }
    }

    /// Every block of `at` (a segment, the container child `seg`), in stretch `run`.
    fn add(&mut self, ids: &PartIds, story: StoryIx, at: &Located<'a>, run: usize, seg: Option<usize>) {
        let first = self.items.len();
        let mut placed: Vec<Option<usize>> = Vec::with_capacity(at.body.blocks.len());
        for (k, block) in at.body.blocks.iter().enumerate() {
            let Some(key) = ids.key_in(story, at, k) else {
                placed.push(None);
                continue;
            };
            let owner = block.owner.and_then(|o| placed.get(o as usize).copied().flatten());
            placed.push(Some(self.items.len()));
            self.by_key.insert(key, self.items.len());
            self.items.push(Item {
                key,
                element: &block.element,
                kind: block.kind,
                run,
                seg,
                owner,
            });
        }
        if let Some(seg) = seg {
            self.by_seg.insert(seg, first..self.items.len());
        }
    }
}

/// One side's blocks as the alignment of blocks equal but for stamps reads them: each
/// block's kind and element in document order, and per container child its blocks.
struct Blocks<'a> {
    items: Vec<(Kind, &'a Arc<Element>)>,
    by_seg: HashMap<usize, Range<usize>>,
}

impl<'a> Blocks<'a> {
    /// A delta side's blocks.
    fn of(side: &Side<'a>) -> Blocks<'a> {
        Blocks {
            items: side.items.iter().map(|i| (i.kind, i.element)).collect(),
            by_seg: side.by_seg.clone(),
        }
    }

    /// Every block of `ids`' part, keyed or not, and each one's place.
    fn every(ids: &'a PartIds) -> (Blocks<'a>, Vec<Path>) {
        let mut out = Blocks {
            items: Vec::new(),
            by_seg: HashMap::new(),
        };
        let mut places = Vec::new();
        let inner = ids.container.len() + 1;
        for at in ids.located() {
            let first = out.items.len();
            for block in at.body.blocks.iter() {
                out.items.push((block.kind, &block.element));
                places.push(PartIds::path_of(&at, block));
            }
            if let Some(seg) = (at.base.len() == inner).then(|| at.base.last()).flatten() {
                out.by_seg.insert(*seg as usize, first..out.items.len());
            }
        }
        (out, places)
    }
}

impl PartIds {
    /// The blocks of this index's part and `new`'s (another version of it, whose blocks may
    /// lack ids: a part another writer put) that are the same but for stamps, as pairs of
    /// places (old, new): aligned in order as a delta pairs a renamed block, the container
    /// children first, then the blocks.
    #[must_use]
    pub fn aligned(&self, new: &PartIds) -> Vec<(Path, Path)> {
        let ((old_blocks, old_places), (new_blocks, new_places)) = (Blocks::every(self), Blocks::every(new));
        let mut pair: Vec<Option<usize>> = vec![None; new_blocks.items.len()];
        let mut taken: Vec<bool> = vec![false; old_blocks.items.len()];
        rename_stamped(&old_blocks, &new_blocks, &mut pair, &mut taken);
        pair.iter()
            .enumerate()
            .filter_map(|(j, i)| Some((old_places.get((*i)?)?.clone(), new_places.get(j)?.clone())))
            .collect()
    }

    /// What differs between this index's part (whose root is `old_root`) and `new`'s, a later
    /// (or any other) version of the same story's part whose root is `new_root`; its blocks
    /// keyed in the story `story`.
    #[must_use]
    pub fn delta(&self, old_root: &Arc<Element>, new: &PartIds, new_root: &Arc<Element>, story: StoryIx) -> StoryDelta {
        if Arc::ptr_eq(old_root, new_root) {
            return StoryDelta::Blocks(BlockDelta::default());
        }
        let (old_at, new_at) = (container(old_root).map(|(k, _)| k), container(new_root).map(|(k, _)| k));
        let same_place = old_at == new_at
            && old_root.name() == new_root.name()
            && *self.container == *new.container
            && self.container.len() == usize::from(old_at.is_some());
        let (Some(old_children), Some(new_children)) = (
            container_children(old_root, old_at),
            container_children(new_root, new_at),
        ) else {
            return StoryDelta::Whole;
        };
        if !same_place || self.segments.len() != old_children.len() || new.segments.len() != new_children.len() {
            return StoryDelta::Whole;
        }
        let (old_c, new_c) = (old_children, new_children);
        let mut runs: Vec<(Range<usize>, Range<usize>)> = ooxml_xml::changed_runs(old_children, new_children)
            .into_iter()
            .map(|r| (r.old, r.new))
            .collect();
        let mut sides = (
            self.side_of(story, &runs, |r| &r.0, new),
            new.side_of(story, &runs, |r| &r.1, self),
        );
        // a block moved past blocks both versions share: the runs between its two places
        // are compared as one, so its order is judged among the blocks it passed
        if let Some(merged) = across(&sides.0, &sides.1, &runs) {
            runs = merged;
            sides = (
                self.side_of(story, &runs, |r| &r.0, new),
                new.side_of(story, &runs, |r| &r.1, self),
            );
        }
        let (old, new_side) = sides;
        let mut delta = compare(&old, &new_side);
        delta.wrapped = wrapped(&old, &new_side, &runs, (old_c, new_c), &delta);
        delta.outside = outside(&runs, (old_c, new_c));
        StoryDelta::Blocks(delta)
    }

    /// The blocks of the container children in each run (`pick`: its old or new stretch),
    /// and the blocks outside the container where they differ from `other`'s.
    fn side_of<'a>(
        &'a self,
        story: StoryIx,
        runs: &[(Range<usize>, Range<usize>)],
        pick: impl Fn(&(Range<usize>, Range<usize>)) -> &Range<usize>,
        other: &PartIds,
    ) -> Side<'a> {
        let mut side = Side::new();
        let last = runs.len() + 1;
        if !same_blocks(&self.before, &other.before) {
            side.before_run.insert(0, None);
            side.add(self, story, &self.outer_before(), 0, None);
        }
        for (n, run) in runs.iter().enumerate() {
            let stretch = pick(run);
            side.before_run
                .insert(n + 1, self.key_before_segment(story, stretch.start));
            let Ok(mut seen) = self.segments.prefix(stretch.start.min(self.segments.len())) else {
                continue;
            };
            for k in stretch.clone() {
                let Some(seg) = self.segments.get(k) else { break };
                if let Some(at) = self.segment(k, &seen) {
                    side.add(self, story, &at, n + 1, Some(k));
                }
                seen = seen.combine(&Count::of(seg));
            }
        }
        if !same_blocks(&self.after, &other.after) {
            side.before_run
                .insert(last, self.key_before_segment(story, self.segments.len()));
            side.add(self, story, &self.outer_after(), last, None);
        }
        side
    }

    /// The key of the last block before container child `k` (`None`: no block is before it).
    fn key_before_segment(&self, story: StoryIx, k: usize) -> Option<BlockKey> {
        for j in (0..k.min(self.segments.len())).rev() {
            let seg = self.segments.get(j)?;
            if seg.body.blocks.is_empty() {
                continue;
            }
            let before = self.segments.prefix(j).ok()?;
            let at = self.segment(j, &before)?;
            return self.key_in(story, &at, at.body.blocks.len() - 1);
        }
        let at = self.outer_before();
        let last = at.body.blocks.len().checked_sub(1)?;
        self.key_in(story, &at, last)
    }
}

/// `runs` with those between the two places of any block held by both sides in different
/// runs merged into one (the container's runs are numbered from 1 in `Item::run`); `None`
/// when no block moved across runs.
fn across(
    old: &Side<'_>,
    new: &Side<'_>,
    runs: &[(Range<usize>, Range<usize>)],
) -> Option<Vec<(Range<usize>, Range<usize>)>> {
    let inner = 1..=runs.len();
    let (mut low, mut high) = (usize::MAX, 0);
    for item in &new.items {
        let Some(was) = old.by_key.get(&item.key).and_then(|i| old.items.get(*i)) else {
            continue;
        };
        if was.run != item.run && inner.contains(&was.run) && inner.contains(&item.run) {
            low = low.min(was.run.min(item.run));
            high = high.max(was.run.max(item.run));
        }
    }
    if low > high {
        return None;
    }
    let (first, last) = (runs.get(low - 1)?, runs.get(high - 1)?);
    let mut out: Vec<(Range<usize>, Range<usize>)> = runs.get(..low - 1)?.to_vec();
    out.push((first.0.start..last.0.end, first.1.start..last.1.end));
    out.extend_from_slice(runs.get(high..)?);
    Some(out)
}

/// Whether two bodies hold the same blocks (the same allocations at the same places).
fn same_blocks(a: &super::segment::Body, b: &super::segment::Body) -> bool {
    a.blocks.len() == b.blocks.len()
        && a.blocks
            .iter()
            .zip(b.blocks.iter())
            .all(|(x, y)| Arc::ptr_eq(&x.element, &y.element) && x.rel == y.rel)
}

/// How a block both versions hold differs.
#[derive(PartialEq, Eq)]
enum Content {
    Same,
    Stamps,
    Changed,
}

fn content(a: &Arc<Element>, b: &Arc<Element>) -> Content {
    if Arc::ptr_eq(a, b) || a == b {
        Content::Same
    } else if stampless_eq(a, b) {
        Content::Stamps
    } else {
        Content::Changed
    }
}

/// The blocks of `old` and `new` compared: pairs by key, then renames.
fn compare(old: &Side<'_>, new: &Side<'_>) -> BlockDelta {
    let (pair, taken) = paired(old, new);
    let renamed: Vec<(usize, usize)> = pair
        .iter()
        .enumerate()
        .filter_map(|(j, i)| {
            let i = (*i)?;
            (old.items.get(i)?.key != new.items.get(j)?.key).then_some((i, j))
        })
        .collect();

    let mut out = BlockDelta::default();
    // the key each old item survives as
    let mut survives: Vec<Option<BlockKey>> = vec![None; old.items.len()];
    for (j, i) in pair.iter().enumerate() {
        if let (Some(i), Some(item)) = (i, new.items.get(j)) {
            if let Some(s) = survives.get_mut(*i) {
                *s = Some(item.key);
            }
        }
    }
    let before_new = |j: usize| -> Option<BlockKey> {
        let item = new.items.get(j)?;
        match j.checked_sub(1).and_then(|p| new.items.get(p)) {
            Some(prev) if prev.run == item.run => Some(prev.key),
            _ => new.before_run.get(&item.run).copied().flatten(),
        }
    };
    let was_after = |i: usize| -> Option<BlockKey> {
        let mut at = i;
        loop {
            let item = old.items.get(at)?;
            match at.checked_sub(1) {
                Some(p) if old.items.get(p).is_some_and(|prev| prev.run == item.run) => {
                    if let Some(key) = survives.get(p).copied().flatten() {
                        return Some(key);
                    }
                    at = p;
                }
                _ => {
                    // the block before the stretch: unchanged unless it is compared too
                    let before = old.before_run.get(&item.run).copied().flatten()?;
                    match old.by_key.get(&before) {
                        Some(&p) => {
                            if let Some(key) = survives.get(p).copied().flatten() {
                                return Some(key);
                            }
                            at = p;
                        }
                        None => return Some(before),
                    }
                }
            }
        }
    };
    for (j, item) in new.items.iter().enumerate() {
        match pair.get(j).copied().flatten() {
            None => out.came.push((item.key, before_new(j))),
            Some(i) => match old.items.get(i).map(|o| content(o.element, item.element)) {
                Some(Content::Stamps) => out.stamp_only.push(item.key),
                Some(Content::Changed) => out.changed.push(item.key),
                _ => {}
            },
        }
    }
    for (i, item) in old.items.iter().enumerate() {
        if !taken.get(i).copied().unwrap_or(false) {
            out.gone.push((item.key, was_after(i)));
        }
    }
    let moving = out_of_order(old, new, &pair);
    out.moved = moved(
        &pair,
        &moving,
        |j| new.items.get(j).map(|i| i.key),
        before_new,
        was_after,
    );
    out.renamed = renamed
        .into_iter()
        .filter_map(|(i, j)| Some((old.items.get(i)?.key, new.items.get(j)?.key)))
        .collect();
    out
}

/// Pairs of items (old, new).
type Pairs = Vec<(usize, usize)>;

/// For each new item, whether its block is out of the order both versions keep: the
/// container children are compared first, by their first blocks (a sequence diff keeps the
/// longest stretches in place, as `touched` has always judged a move among the body's
/// blocks), a block moving with its child or away from it; the rest by a sequence diff of
/// the blocks themselves.
fn out_of_order(old: &Side<'_>, new: &Side<'_>, pair: &[Option<usize>]) -> Vec<bool> {
    let mut out = vec![false; new.items.len()];
    let pairs: Pairs = pair.iter().enumerate().filter_map(|(j, i)| i.map(|i| (i, j))).collect();
    let first_of = |i: usize| {
        let seg = old.items.get(i)?.seg?;
        (old.by_seg.get(&seg)?.start == i).then_some(seg)
    };
    let mut firsts: Pairs = pairs.iter().copied().filter(|(i, _)| first_of(*i).is_some()).collect();
    firsts.sort_unstable();
    let kept = kept_in_order(&firsts.iter().map(|(_, j)| *j).collect::<Vec<_>>());
    // per old container child: whether it kept its place, and where its first block went
    let children: HashMap<usize, (bool, Option<usize>)> = firsts
        .iter()
        .zip(kept)
        .filter_map(|((i, j), kept)| Some((first_of(*i)?, (kept, new.items.get(*j)?.seg))))
        .collect();
    let mut rest: Pairs = Vec::new();
    for (i, j) in pairs {
        let (Some(o), Some(n)) = (old.items.get(i), new.items.get(j)) else {
            continue;
        };
        let moving = match o.seg.and_then(|s| children.get(&s)) {
            Some((false, _)) => true,
            Some((true, to)) if *to != n.seg => true,
            _ => {
                rest.push((i, j));
                false
            }
        };
        if let Some(flag) = out.get_mut(j) {
            *flag = moving;
        }
    }
    rest.sort_unstable();
    let kept = kept_in_order(&rest.iter().map(|(_, j)| *j).collect::<Vec<_>>());
    for ((_, j), kept) in rest.iter().zip(kept) {
        if let Some(flag) = out.get_mut(*j) {
            *flag = !kept;
        }
    }
    out
}

/// The paired blocks `moving` marks, in the later version's order.
fn moved(
    pair: &[Option<usize>],
    moving: &[bool],
    key: impl Fn(usize) -> Option<BlockKey>,
    before_new: impl Fn(usize) -> Option<BlockKey>,
    was_after: impl Fn(usize) -> Option<BlockKey>,
) -> Vec<Moved> {
    pair.iter()
        .enumerate()
        .filter(|(j, _)| moving.get(*j).copied().unwrap_or(false))
        .filter_map(|(j, i)| {
            Some(Moved {
                key: key(j)?,
                after: before_new(j),
                was_after: was_after((*i)?),
            })
        })
        .collect()
}

/// The old item each new item is paired with -- by key, then as a renamed table or a
/// stamped block -- and which old items are paired.
fn paired(old: &Side<'_>, new: &Side<'_>) -> (Vec<Option<usize>>, Vec<bool>) {
    let mut pair: Vec<Option<usize>> = vec![None; new.items.len()];
    let mut taken: Vec<bool> = vec![false; old.items.len()];
    for (j, item) in new.items.iter().enumerate() {
        let Some(&i) = old.by_key.get(&item.key) else { continue };
        if pair.get(j).copied().flatten().is_none() && !taken.get(i).copied().unwrap_or(true) {
            link(&mut pair, &mut taken, i, j);
        }
    }
    rename_tables(old, new, &mut pair, &mut taken);
    rename_stamped(&Blocks::of(old), &Blocks::of(new), &mut pair, &mut taken);
    (pair, taken)
}

/// A came table paired with a gone one that held one of its rows (its first row changed).
fn rename_tables(old: &Side<'_>, new: &Side<'_>, pair: &mut [Option<usize>], taken: &mut [bool]) {
    let mut rows: HashMap<usize, Vec<usize>> = HashMap::new();
    for (j, item) in new.items.iter().enumerate() {
        if let (Kind::Tr, Some(owner)) = (item.kind, item.owner) {
            rows.entry(owner).or_default().push(j);
        }
    }
    for (j, item) in new.items.iter().enumerate() {
        if item.kind != Kind::Tbl || pair.get(j).copied().flatten().is_some() {
            continue;
        }
        let gone_table = rows.get(&j).into_iter().flatten().find_map(|row| {
            let i = pair.get(*row).copied().flatten()?;
            let table = old.items.get(i)?.owner?;
            (!taken.get(table).copied().unwrap_or(true)).then_some(table)
        });
        if let Some(i) = gone_table {
            link(pair, taken, i, j);
        }
    }
}

/// A came block paired with a gone one of its kind whose content is the same but for stamps
/// (stamping gave it a paraId, an undo of stamping took it away, a block was replaced by a
/// copy of itself under new ids): the two sides' unpaired blocks aligned in order by a
/// sequence diff, as `touched` has always found such blocks unchanged.
fn rename_stamped(old: &Blocks<'_>, new: &Blocks<'_>, pair: &mut [Option<usize>], taken: &mut [bool]) {
    let mut memo: HashMap<*const Element, u64> = HashMap::new();
    // the container children first, whose blocks are all unpaired, by their blocks' digests
    // (difflib over the body's blocks, as `touched` judged them): a moved section is a
    // removal and an insertion there, not a stretch of its many nested blocks kept
    let old_free = |i: usize| !taken.get(i).copied().unwrap_or(true);
    let new_free = |j: usize, pair: &[Option<usize>]| pair.get(j).copied().flatten().is_none();
    let children = |side: &Blocks<'_>, free: &dyn Fn(usize) -> bool, memo: &mut HashMap<*const Element, u64>| {
        let mut segs: Vec<(usize, Range<usize>)> = side
            .by_seg
            .iter()
            .filter(|(_, r)| !r.is_empty() && Range::clone(r).all(free))
            .map(|(k, r)| (*k, r.clone()))
            .collect();
        segs.sort_by_key(|(k, _)| *k);
        let digests: Vec<u64> = segs
            .iter()
            .map(|(_, r)| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for (kind, element) in side.items.get(r.clone()).unwrap_or_default() {
                    (kind_tag(*kind), stampless_hash(element, memo)).hash(&mut h);
                }
                h.finish()
            })
            .collect();
        (segs, digests)
    };
    let (old_segs, a) = children(old, &old_free, &mut memo);
    let snapshot: Vec<Option<usize>> = pair.to_vec();
    let (new_segs, b) = children(new, &|j| new_free(j, &snapshot), &mut memo);
    for op in opcodes(&a, &b).into_iter().filter(|op| op.tag == Tag::Equal) {
        for (x, y) in (op.i1..op.i2).zip(op.j1..op.j2) {
            let (Some((_, ra)), Some((_, rb))) = (old_segs.get(x), new_segs.get(y)) else {
                continue;
            };
            if ra.len() == rb.len() {
                for (i, j) in ra.clone().zip(rb.clone()) {
                    link_alike(old, new, pair, taken, i, j);
                }
            }
        }
    }
    // then the blocks still unpaired, each side in order, by kind and content without stamps
    let digests =
        |items: &[(Kind, &Arc<Element>)], keep: &dyn Fn(usize) -> bool, memo: &mut HashMap<*const Element, u64>| {
            items
                .iter()
                .enumerate()
                .filter(|(n, _)| keep(*n))
                .map(|(n, (kind, element))| (n, (kind_tag(*kind), stampless_hash(element, memo))))
                .unzip::<usize, (u8, u64), Vec<usize>, Vec<(u8, u64)>>()
        };
    let (olds, a) = digests(&old.items, &|i| !taken.get(i).copied().unwrap_or(true), &mut memo);
    let (news, b) = digests(&new.items, &|j| pair.get(j).copied().flatten().is_none(), &mut memo);
    for op in opcodes(&a, &b).into_iter().filter(|op| op.tag == Tag::Equal) {
        for (x, y) in (op.i1..op.i2).zip(op.j1..op.j2) {
            if let (Some(&i), Some(&j)) = (olds.get(x), news.get(y)) {
                link_alike(old, new, pair, taken, i, j);
            }
        }
    }
}

/// `i` and `j` paired where their blocks are the same but for stamps.
fn link_alike(old: &Blocks<'_>, new: &Blocks<'_>, pair: &mut [Option<usize>], taken: &mut [bool], i: usize, j: usize) {
    let alike = old
        .items
        .get(i)
        .zip(new.items.get(j))
        .is_some_and(|((ok, oe), (nk, ne))| ok == nk && stampless_eq(oe, ne));
    if alike {
        link(pair, taken, i, j);
    }
}

fn link(pair: &mut [Option<usize>], taken: &mut [bool], i: usize, j: usize) {
    if let Some(p) = pair.get_mut(j) {
        *p = Some(i);
    }
    if let Some(t) = taken.get_mut(i) {
        *t = true;
    }
}

fn kind_tag(kind: Kind) -> u8 {
    match kind {
        Kind::P => 0,
        Kind::Tr => 1,
        Kind::Tbl => 2,
    }
}

/// For each place of `seq` (the later version's places of the paired blocks, in the earlier
/// version's order), whether its block kept its order: in a matching block of a sequence diff
/// of `seq` against itself sorted -- the blocks a move left in place are those `difflib`
/// matches, as `touched` has always found them.
fn kept_in_order(seq: &[usize]) -> Vec<bool> {
    let mut sorted = seq.to_vec();
    sorted.sort_unstable();
    let mut kept = vec![false; seq.len()];
    for op in opcodes(seq, &sorted).into_iter().filter(|op| op.tag == Tag::Equal) {
        for k in kept.get_mut(op.i1..op.i2).into_iter().flatten() {
            *k = true;
        }
    }
    kept
}

/// Whether a container child is a block-level element (a paragraph, table, content control
/// or custom XML): what a story's blocks live in.
fn block_level(node: &Node) -> Option<&Arc<Element>> {
    node.as_element()
        .filter(|e| e.name().namespace() == Some(W) && matches!(e.name().local(), "p" | "tbl" | "sdt" | "customXml"))
}

/// The blocks that did not change themselves whose block-level container child differs from
/// the one that held them before, named by the first block inside.
fn wrapped(
    old: &Side<'_>,
    new: &Side<'_>,
    runs: &[(Range<usize>, Range<usize>)],
    (old_c, new_c): (&Children, &Children),
    delta: &BlockDelta,
) -> Vec<BlockKey> {
    let marked: std::collections::HashSet<BlockKey> = delta
        .changed
        .iter()
        .copied()
        .chain(delta.came.iter().map(|(k, _)| *k))
        .collect();
    let renamed_from = delta.renamed_from();
    let mut out = Vec::new();
    for (_, stretch) in runs {
        for k in stretch.clone() {
            let Some(child) = new_c.get(k).and_then(block_level) else {
                continue;
            };
            let Some(items) = new.by_seg.get(&k) else { continue };
            let mut blocks = new.items.get(items.clone()).unwrap_or_default().iter();
            let Some(first) = blocks.next() else { continue };
            if std::iter::once(first).chain(blocks).any(|i| marked.contains(&i.key)) {
                continue;
            }
            // the container child that held the first block before (under its old key)
            let old_key = renamed_from.get(&first.key).copied().unwrap_or(first.key);
            let Some(was) = old
                .by_key
                .get(&old_key)
                .and_then(|i| old.items.get(*i))
                .and_then(|i| i.seg)
                .and_then(|s| old_c.get(s))
                .and_then(Node::as_element)
            else {
                continue;
            };
            if !stampless_eq(was, child) {
                out.push(first.key);
            }
        }
    }
    out
}

/// Whether the container's markup outside its blocks differs within the runs: the children
/// that are not block-level, or a block-level child that holds no block and is new.
fn outside(runs: &[(Range<usize>, Range<usize>)], (old_c, new_c): (&Children, &Children)) -> bool {
    runs.iter().any(|(a, b)| {
        let gaps = |c: &Children, r: &Range<usize>| -> Vec<Node> {
            r.clone()
                .filter_map(|k| c.get(k))
                .filter(|n| block_level(n).is_none())
                .cloned()
                .collect()
        };
        if gaps(old_c, a) != gaps(new_c, b) {
            return true;
        }
        b.clone()
            .filter_map(|k| new_c.get(k).and_then(block_level))
            .filter(|e| !holds_block(e))
            .any(|e| !a.clone().any(|k| old_c.get(k).and_then(Node::as_element) == Some(e)))
    })
}

/// Whether `element` holds a paragraph or table (live: in the branch of an
/// `mc:AlternateContent` the readers read, [`crate::mc::chosen`]).
fn holds_block(element: &Element) -> bool {
    if element.name().namespace() == Some(W) && matches!(element.name().local(), "p" | "tbl") {
        return true;
    }
    element.children().iter().any(|n| {
        n.as_element()
            .is_some_and(|e| !crate::mc::unchosen(element, e) && holds_block(e))
    })
}

/// Whether `name` is an attribute the ids stamping writes.
fn stamp(name: &ooxml_xml::Name) -> bool {
    (name.namespace() == Some(W14) && matches!(name.local(), "paraId" | "textId"))
        || (name.namespace() == Some(WP14) && matches!(name.local(), "anchorId" | "editId"))
}

/// Whether `a` and `b` are equal but for the attributes the ids stamping writes.
fn stampless_eq(a: &Arc<Element>, b: &Arc<Element>) -> bool {
    if Arc::ptr_eq(a, b) {
        return true;
    }
    if a.name() != b.name() || a.namespaces() != b.namespaces() {
        return false;
    }
    let attributes = |e: &Element| {
        e.attributes()
            .iter()
            .filter(|at| !stamp(&at.name))
            .map(|at| (at.name.clone(), at.value.clone()))
            .collect::<Vec<_>>()
    };
    if attributes(a) != attributes(b) || a.children().len() != b.children().len() {
        return false;
    }
    a.children().iter().zip(b.children().iter()).all(|(x, y)| match (x, y) {
        (Node::Element(x), Node::Element(y)) => stampless_eq(x, y),
        _ => x == y,
    })
}

/// A digest of `element` without the stamps' attributes (equal for [`stampless_eq`] ones),
/// memoised by allocation for this comparison only.
fn stampless_hash(element: &Arc<Element>, memo: &mut HashMap<*const Element, u64>) -> u64 {
    if let Some(found) = memo.get(&Arc::as_ptr(element)) {
        return *found;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    element.name().namespace().hash(&mut h);
    element.name().local().hash(&mut h);
    for at in element.attributes().iter().filter(|at| !stamp(&at.name)) {
        at.name.namespace().hash(&mut h);
        at.name.local().hash(&mut h);
        at.value.hash(&mut h);
    }
    for node in element.children().iter() {
        match node {
            Node::Element(child) => {
                1u8.hash(&mut h);
                stampless_hash(child, memo).hash(&mut h);
            }
            Node::Text(t) | Node::Comment(t) | Node::EntityRef(t) => {
                2u8.hash(&mut h);
                t.hash(&mut h);
            }
            Node::Pi(p) => {
                3u8.hash(&mut h);
                p.target.hash(&mut h);
                p.data.hash(&mut h);
            }
        }
    }
    let digest = h.finish();
    memo.insert(Arc::as_ptr(element), digest);
    digest
}
