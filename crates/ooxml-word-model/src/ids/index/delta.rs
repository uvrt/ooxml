//! Owns: a story's change between two versions by block identity ([`PartIds::delta`]): the
//! container children the later version does not share with the earlier one are found by
//! identity (`ooxml_xml::changed_runs`), and only their blocks are keyed and compared -- by
//! key, then by content (the same allocation, equal, or equal but for the ids stamping
//! writes) -- the rest of the story being the same blocks under the same keys. Where either
//! version's keys do not follow from content alone (a block without a paraId of its own, a
//! repeat, a short spelling: an unstamped version), every block is compared, since a key
//! there names a place.
//!
//! Renames pair a gone block with a came one: a table with the table sharing one of its
//! rows (its first row changed), and a block whose key named its place with the block of
//! the same content but stamps (stamping gave it a paraId).
//!
//! Must not: key identity by an element's address (a pointer only says content is the same,
//! ADR-008), or name a block by text.

use super::build::{container, container_children};
use super::segment::Kind;
use super::{Count, Located, PartIds};
use crate::ids::delta::{BlockDelta, Moved, StoryDelta};
use crate::ids::key::{BlockKey, StoryIx};
use crate::ids::{W, W14, WP14};
use ooxml_derive::Monoid;
use ooxml_xml::{Children, Element, Node};
use std::collections::{HashMap, VecDeque};
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

/// One side's blocks compared, in document order, with each stretch's first item and the
/// block before the stretch (outside the compared blocks, or `None`).
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

impl PartIds {
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
        let runs: Vec<(Range<usize>, Range<usize>)> = ooxml_xml::changed_runs(old_children, new_children)
            .into_iter()
            .map(|r| (r.old, r.new))
            .collect();
        let by_content = self.keys_by_content() && new.keys_by_content();
        let (old, new_side) = if by_content {
            (
                self.side_of(story, &runs, |r| &r.0, new),
                new.side_of(story, &runs, |r| &r.1, self),
            )
        } else {
            (self.side_whole(story), new.side_whole(story))
        };
        // where keys name places, the blocks the two versions share are paired by place first
        let shared = if by_content {
            Vec::new()
        } else {
            self.shared_blocks(new, (&old, &new_side), &runs)
        };
        let mut delta = compare(&old, &new_side, &shared);
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

    /// The blocks both sides hold in the container children (and outer blocks) the two
    /// versions share, as pairs of items (old, new): each such child's blocks in order.
    fn shared_blocks(
        &self,
        new: &PartIds,
        (old_side, new_side): (&Side<'_>, &Side<'_>),
        runs: &[(Range<usize>, Range<usize>)],
    ) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut zip = |a: Range<usize>, b: Range<usize>| {
            if a.len() == b.len() {
                out.extend(a.zip(b));
            }
        };
        let (old_n, new_n) = (old_side.items.len(), new_side.items.len());
        if same_blocks(&self.before, &new.before) {
            let n = self.before.blocks.len();
            zip(0..n.min(old_n), 0..n.min(new_n));
        }
        if same_blocks(&self.after, &new.after) {
            let n = self.after.blocks.len();
            zip(old_n.saturating_sub(n)..old_n, new_n.saturating_sub(n)..new_n);
        }
        // the children between runs: the same allocations, shifted by the runs before them
        let mut from = 0;
        let mut shift: isize = 0;
        let mut stretches: Vec<(Range<usize>, isize)> = Vec::new();
        for (a, b) in runs {
            stretches.push((from..a.start, shift));
            shift = b.end as isize - a.end as isize;
            from = a.end;
        }
        stretches.push((from..self.segments.len(), shift));
        for (stretch, shift) in stretches {
            for k in stretch {
                let Some(n) = k.checked_add_signed(shift) else { continue };
                if let (Some(a), Some(b)) = (old_side.by_seg.get(&k), new_side.by_seg.get(&n)) {
                    zip(a.clone(), b.clone());
                }
            }
        }
        out
    }

    /// Every block of the part, as one stretch.
    fn side_whole(&self, story: StoryIx) -> Side<'_> {
        let mut side = Side::new();
        side.before_run.insert(0, None);
        let inner = self.container.len() + 1;
        for at in self.located() {
            let seg = (at.base.len() == inner)
                .then(|| at.base.last().map(|k| *k as usize))
                .flatten();
            side.add(self, story, &at, 0, seg);
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
fn compare(old: &Side<'_>, new: &Side<'_>, shared: &[(usize, usize)]) -> BlockDelta {
    // the old item each new item is paired with: by place where given, then by key
    let mut pair: Vec<Option<usize>> = vec![None; new.items.len()];
    let mut taken: Vec<bool> = vec![false; old.items.len()];
    for (i, j) in shared {
        link(&mut pair, &mut taken, *i, *j);
    }
    for (j, item) in new.items.iter().enumerate() {
        let Some(&i) = old.by_key.get(&item.key) else { continue };
        if pair.get(j).copied().flatten().is_none() && !taken.get(i).copied().unwrap_or(true) {
            link(&mut pair, &mut taken, i, j);
        }
    }
    rename_tables(old, new, &mut pair, &mut taken);
    rename_stamped(old, new, &mut pair, &mut taken);
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
    let crossed = |i: usize, j: usize| old.items.get(i).map(|o| o.run) != new.items.get(j).map(|n| n.run);
    out.moved = moved(
        &pair,
        crossed,
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

/// The paired blocks that crossed blocks both versions share (`crossed`: from one stretch
/// compared to another), and those outside a longest stretch whose order both versions
/// keep, in the later version's order.
fn moved(
    pair: &[Option<usize>],
    crossed: impl Fn(usize, usize) -> bool,
    key: impl Fn(usize) -> Option<BlockKey>,
    before_new: impl Fn(usize) -> Option<BlockKey>,
    was_after: impl Fn(usize) -> Option<BlockKey>,
) -> Vec<Moved> {
    let (mut order, across): (Vec<(usize, usize)>, Vec<(usize, usize)>) = pair
        .iter()
        .enumerate()
        .filter_map(|(j, i)| i.map(|i| (i, j)))
        .partition(|(i, j)| !crossed(*i, *j));
    order.sort_unstable();
    let kept = longest_increasing(&order.iter().map(|(_, j)| *j).collect::<Vec<_>>());
    let mut out: Vec<(usize, Moved)> = order
        .iter()
        .zip(kept)
        .filter(|(_, kept)| !kept)
        .map(|(pair, _)| pair)
        .chain(&across)
        .filter_map(|(i, j)| {
            Some((
                *j,
                Moved {
                    key: key(*j)?,
                    after: before_new(*j),
                    was_after: was_after(*i),
                },
            ))
        })
        .collect();
    out.sort_by_key(|(j, _)| *j);
    out.into_iter().map(|(_, m)| m).collect()
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

/// A came block paired with a gone one of its kind whose key named its place and whose
/// content is the same but for stamps (stamping gave it a paraId of its own), in order.
fn rename_stamped(old: &Side<'_>, new: &Side<'_>, pair: &mut [Option<usize>], taken: &mut [bool]) {
    let unpaired_old = old
        .items
        .iter()
        .enumerate()
        .filter(|(i, _)| !taken.get(*i).copied().unwrap_or(true));
    let unpaired_new = new
        .items
        .iter()
        .enumerate()
        .filter(|(j, _)| pair.get(*j).copied().flatten().is_none());
    if unpaired_old.chain(unpaired_new).all(|(_, item)| item.key.durable()) {
        return; // two durable keys are two blocks
    }
    let mut memo: HashMap<*const Element, u64> = HashMap::new();
    let mut waiting: HashMap<(u8, u64), VecDeque<usize>> = HashMap::new();
    for (i, item) in old.items.iter().enumerate() {
        if taken.get(i).copied().unwrap_or(true) {
            continue;
        }
        let digest = stampless_hash(item.element, &mut memo);
        waiting.entry((kind_tag(item.kind), digest)).or_default().push_back(i);
    }
    if waiting.is_empty() {
        return;
    }
    for (j, item) in new.items.iter().enumerate() {
        if pair.get(j).copied().flatten().is_some() {
            continue;
        }
        let digest = stampless_hash(item.element, &mut memo);
        let Some(queue) = waiting.get_mut(&(kind_tag(item.kind), digest)) else {
            continue;
        };
        let found = queue.iter().position(|i| {
            !taken.get(*i).copied().unwrap_or(true)
                && old
                    .items
                    .get(*i)
                    .is_some_and(|o| !(o.key.durable() && item.key.durable()) && stampless_eq(o.element, item.element))
        });
        if let Some(i) = found.and_then(|p| queue.remove(p)) {
            link(pair, taken, i, j);
        }
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

/// For each place of `seq`, whether it is in a longest strictly increasing subsequence.
fn longest_increasing(seq: &[usize]) -> Vec<bool> {
    // patience: tails[l] is the place of the smallest tail of an increasing run of length l+1
    let mut tails: Vec<usize> = Vec::new();
    let mut back: Vec<Option<usize>> = vec![None; seq.len()];
    for (n, v) in seq.iter().enumerate() {
        let l = tails.partition_point(|t| seq.get(*t).is_some_and(|x| x < v));
        if let Some(b) = back.get_mut(n) {
            *b = l.checked_sub(1).and_then(|p| tails.get(p).copied());
        }
        if l == tails.len() {
            tails.push(n);
        } else if let Some(t) = tails.get_mut(l) {
            *t = n;
        }
    }
    let mut kept = vec![false; seq.len()];
    let mut at = tails.last().copied();
    while let Some(n) = at {
        if let Some(k) = kept.get_mut(n) {
            *k = true;
        }
        at = back.get(n).copied().flatten();
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
            let old_key = delta
                .renamed
                .iter()
                .find(|(_, n)| *n == first.key)
                .map_or(first.key, |(o, _)| *o);
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

/// Whether `element` holds a paragraph or table (live, not under `mc:Fallback`).
fn holds_block(element: &Element) -> bool {
    if element.name().namespace() == Some(W) && matches!(element.name().local(), "p" | "tbl") {
        return true;
    }
    element.children().iter().any(|n| {
        n.as_element()
            .is_some_and(|e| !e.name().is(Some(crate::ids::MC), "Fallback") && holds_block(e))
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
