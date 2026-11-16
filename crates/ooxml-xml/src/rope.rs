//! Owns: [`Children`], an element's child list as a persistent B-tree rope (ADR-003): leaves
//! of up to [`LEAF_MAX`] nodes, branches of up to [`BRANCH_MAX`] chunks, each branch knowing
//! its chunks' lengths. An element of at most `LEAF_MAX` children is one leaf (a plain
//! shared slice); a wide one (a body of 29,000 blocks) is a tree, so an edit copies one
//! leaf and the branches above it, and shares the rest.
//!
//! Must not: hold anything but nodes, or mutate a chunk once built.

use crate::node::Node;
use std::ops::Range;
use std::sync::Arc;

/// Most nodes in a leaf.
pub const LEAF_MAX: usize = 64;
/// Most chunks in a branch.
pub const BRANCH_MAX: usize = 32;
/// A leaf or branch smaller than this is merged with a neighbour after an edit when the two
/// fit in one.
const MERGE_BELOW: usize = 8;

/// An element's children: a persistent sequence of nodes.
#[derive(Debug, Clone, Default)]
pub struct Children(Chunk);

#[derive(Debug, Clone, Default)]
enum Chunk {
    #[default]
    Empty,
    Leaf(Arc<[Node]>),
    Branch(Arc<Branch>),
}

#[derive(Debug)]
struct Branch {
    len: usize,
    /// `sizes[i]` is `kids[i].len()`.
    sizes: Box<[usize]>,
    kids: Box<[Chunk]>,
}

impl Chunk {
    fn len(&self) -> usize {
        match self {
            Chunk::Empty => 0,
            Chunk::Leaf(l) => l.len(),
            Chunk::Branch(b) => b.len,
        }
    }

    fn branch(kids: Vec<Chunk>) -> Chunk {
        let sizes: Box<[usize]> = kids.iter().map(Chunk::len).collect();
        Chunk::Branch(Arc::new(Branch { len: sizes.iter().sum(), sizes, kids: kids.into() }))
    }

    fn width(&self) -> usize {
        match self {
            Chunk::Empty => 0,
            Chunk::Leaf(l) => l.len(),
            Chunk::Branch(b) => b.kids.len(),
        }
    }

    /// One chunk holding `a` then `b` (both of one height), when they fit in one.
    fn merged(a: &Chunk, b: &Chunk) -> Option<Chunk> {
        match (a, b) {
            (Chunk::Leaf(x), Chunk::Leaf(y)) if x.len() + y.len() <= LEAF_MAX => {
                Some(Chunk::Leaf(x.iter().chain(y.iter()).cloned().collect()))
            }
            (Chunk::Branch(x), Chunk::Branch(y)) if x.kids.len() + y.kids.len() <= BRANCH_MAX => {
                Some(Chunk::branch(x.kids.iter().chain(y.kids.iter()).cloned().collect()))
            }
            _ => None,
        }
    }
}

/// `n` items in `ceil(n / max)` groups of near-equal size.
fn balanced<T>(items: Vec<T>, max: usize, make: impl Fn(Vec<T>) -> Chunk) -> Vec<Chunk> {
    if items.is_empty() {
        return Vec::new();
    }
    let groups = items.len().div_ceil(max);
    let (base, extra) = (items.len() / groups, items.len() % groups);
    let mut out = Vec::with_capacity(groups);
    let mut rest = items.into_iter();
    for g in 0..groups {
        let take = base + usize::from(g < extra);
        out.push(make(rest.by_ref().take(take).collect()));
    }
    out
}

fn leaves(nodes: Vec<Node>) -> Vec<Chunk> {
    balanced(nodes, LEAF_MAX, |v| Chunk::Leaf(v.into()))
}

/// Chunks of one height under as few new levels as it takes to make one root.
fn root_of(mut level: Vec<Chunk>) -> Chunk {
    loop {
        match level.len() {
            0 => return Chunk::Empty,
            1 => {
                let only = level.pop().unwrap_or_default();
                return collapse(only);
            }
            _ => level = balanced(level, BRANCH_MAX, Chunk::branch),
        }
    }
}

/// A branch of one chunk is that chunk.
fn collapse(mut chunk: Chunk) -> Chunk {
    while let Chunk::Branch(b) = &chunk {
        match b.kids.first() {
            Some(only) if b.kids.len() == 1 => chunk = only.clone(),
            _ => break,
        }
    }
    chunk
}

/// Merge underfull neighbours in `kids[lo..hi]` (the chunks an edit made, plus one on each
/// side).
fn merge_small(kids: &mut Vec<Chunk>, lo: usize, hi: usize) {
    let mut i = lo;
    while i + 1 < hi.min(kids.len()) {
        let (Some(a), Some(b)) = (kids.get(i), kids.get(i + 1)) else { break };
        if a.width() < MERGE_BELOW || b.width() < MERGE_BELOW {
            if let Some(m) = Chunk::merged(a, b) {
                kids.splice(i..i + 2, [m]);
                continue;
            }
        }
        i += 1;
    }
}

/// `chunk` with `range` replaced by `with` (taken by the first chunk that reaches the
/// range's start): zero or more chunks of the same height.
fn splice(chunk: &Chunk, range: Range<usize>, with: &mut Option<Vec<Node>>) -> Vec<Chunk> {
    match chunk {
        Chunk::Empty => leaves(with.take().unwrap_or_default()),
        Chunk::Leaf(l) => {
            let mut items: Vec<Node> = Vec::with_capacity(l.len() + with.as_ref().map_or(0, Vec::len));
            items.extend(l.iter().take(range.start).cloned());
            items.extend(with.take().unwrap_or_default());
            items.extend(l.iter().skip(range.end).cloned());
            leaves(items)
        }
        Chunk::Branch(b) => splice_branch(b, range, with),
    }
}

fn splice_branch(b: &Branch, range: Range<usize>, with: &mut Option<Vec<Node>>) -> Vec<Chunk> {
    let last = b.kids.len().saturating_sub(1);
    let mut kids: Vec<Chunk> = Vec::with_capacity(b.kids.len() + 2);
    let (mut first_new, mut after_new) = (usize::MAX, 0);
    let mut offset = 0;
    for (k, (kid, &size)) in b.kids.iter().zip(b.sizes.iter()).enumerate() {
        let end = offset + size;
        // the kid holding the range's start takes the insertion; a kid inside the range
        // loses its part of it; the rest stay as they are
        let takes_start = with.is_some() && (range.start < end || (k == last && range.start == end));
        let overlaps = range.start < end && range.end > offset;
        if takes_start || overlaps {
            let start = range.start.saturating_sub(offset).min(size);
            let local = start..range.end.min(end).saturating_sub(offset).max(start);
            first_new = first_new.min(kids.len());
            if !takes_start && local.start == 0 && local.end == size {
                // wholly removed
            } else {
                kids.extend(splice(kid, local, with));
            }
            after_new = kids.len();
        } else {
            kids.push(kid.clone());
        }
        offset = end;
    }
    if first_new != usize::MAX {
        merge_small(&mut kids, first_new.saturating_sub(1), after_new + 1);
    }
    balanced(kids, BRANCH_MAX, Chunk::branch)
}

impl Children {
    /// Children from a list of nodes.
    pub fn from_vec(nodes: Vec<Node>) -> Children {
        Children(root_of(leaves(nodes)))
    }

    /// Children taken from `kids[start..]` (one allocation for a narrow element).
    pub(crate) fn from_drain(kids: &mut Vec<Node>, start: usize) -> Children {
        let n = kids.len().saturating_sub(start);
        match n {
            0 => Children(Chunk::Empty),
            _ if n <= LEAF_MAX => Children(Chunk::Leaf(kids.drain(start..).collect())),
            _ => Children::from_vec(kids.split_off(start)),
        }
    }

    /// No children.
    pub fn empty() -> Children {
        Children(Chunk::Empty)
    }

    /// How many children.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The child at `index`.
    pub fn get(&self, mut index: usize) -> Option<&Node> {
        let mut chunk = &self.0;
        loop {
            match chunk {
                Chunk::Empty => return None,
                Chunk::Leaf(l) => return l.get(index),
                Chunk::Branch(b) => {
                    let mut found = None;
                    for (kid, &size) in b.kids.iter().zip(b.sizes.iter()) {
                        if index < size {
                            found = Some(kid);
                            break;
                        }
                        index -= size;
                    }
                    chunk = found?;
                }
            }
        }
    }

    /// The children in order.
    pub fn iter(&self) -> Iter<'_> {
        let mut it = Iter { stack: Vec::new(), leaf: [].iter() };
        it.descend(&self.0);
        it
    }

    /// The children as a vector.
    pub fn to_vec(&self) -> Vec<Node> {
        self.iter().cloned().collect()
    }

    /// New children: these with `range` replaced by `with`; the chunks outside the edit are
    /// shared. `None` when `range` is not within `0..len()`.
    pub fn splice(&self, range: Range<usize>, with: Vec<Node>) -> Option<Children> {
        if range.start > range.end || range.end > self.len() {
            return None;
        }
        let mut with = Some(with);
        let level = splice(&self.0, range, &mut with);
        Some(Children(root_of(level)))
    }

    /// Whether both are the same allocation (then they are equal without looking).
    pub fn ptr_eq(&self, other: &Children) -> bool {
        match (&self.0, &other.0) {
            (Chunk::Empty, Chunk::Empty) => true,
            (Chunk::Leaf(a), Chunk::Leaf(b)) => Arc::ptr_eq(a, b),
            (Chunk::Branch(a), Chunk::Branch(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// The tree's height (0 for one leaf): for tests and measurements.
    pub fn height(&self) -> usize {
        let mut h = 0;
        let mut chunk = &self.0;
        while let Chunk::Branch(b) = chunk {
            h += 1;
            match b.kids.first() {
                Some(k) => chunk = k,
                None => break,
            }
        }
        h
    }
}

impl PartialEq for Children {
    fn eq(&self, other: &Children) -> bool {
        self.ptr_eq(other) || (self.len() == other.len() && self.iter().zip(other.iter()).all(|(a, b)| a == b))
    }
}

impl Eq for Children {}

impl FromIterator<Node> for Children {
    fn from_iter<I: IntoIterator<Item = Node>>(iter: I) -> Self {
        Children::from_vec(iter.into_iter().collect())
    }
}

impl<'a> IntoIterator for &'a Children {
    type Item = &'a Node;
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

/// An iterator over [`Children`].
#[derive(Debug, Clone)]
pub struct Iter<'a> {
    stack: Vec<std::slice::Iter<'a, Chunk>>,
    leaf: std::slice::Iter<'a, Node>,
}

impl<'a> Iter<'a> {
    fn descend(&mut self, mut chunk: &'a Chunk) {
        loop {
            match chunk {
                Chunk::Empty => return,
                Chunk::Leaf(l) => {
                    self.leaf = l.iter();
                    return;
                }
                Chunk::Branch(b) => {
                    let mut kids = b.kids.iter();
                    match kids.next() {
                        Some(first) => {
                            self.stack.push(kids);
                            chunk = first;
                        }
                        None => return,
                    }
                }
            }
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a Node;
    fn next(&mut self) -> Option<&'a Node> {
        loop {
            if let Some(n) = self.leaf.next() {
                return Some(n);
            }
            let top = self.stack.last_mut()?;
            match top.next() {
                Some(chunk) => self.descend(chunk),
                None => {
                    self.stack.pop();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(i: usize) -> Node {
        Node::Text(i.to_string().into())
    }

    fn values(c: &Children) -> Vec<String> {
        c.iter().map(|n| n.as_text().unwrap().to_owned()).collect()
    }

    /// A small deterministic generator (xorshift), so the test needs no dependency.
    struct Rng(u64);
    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % (n as u64).max(1)) as usize
        }
    }

    #[test]
    fn splices_equal_a_vector_model() {
        for seed in 1..40u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let n = rng.below(3000);
            let mut model: Vec<Node> = (0..n).map(text).collect();
            let mut rope = Children::from_vec(model.clone());
            let mut next = n;
            for _ in 0..200 {
                let len = model.len();
                let a = rng.below(len + 1);
                let wide = rng.below(4) == 0;
                let b = (a + rng.below(if wide { 400 } else { 4 })).min(len);
                let many = rng.below(4) == 0;
                let k = rng.below(if many { 300 } else { 6 });
                let with: Vec<Node> = (next..next + k).map(text).collect();
                next += k;
                rope = rope.splice(a..b, with.clone()).unwrap();
                model.splice(a..b, with);
                assert_eq!(rope.len(), model.len());
            }
            assert_eq!(values(&rope), model.iter().map(|n| n.as_text().unwrap().to_owned()).collect::<Vec<_>>());
            for (i, node) in model.iter().enumerate() {
                assert_eq!(rope.get(i), Some(node));
            }
            assert_eq!(rope.get(model.len()), None);
        }
    }

    #[test]
    fn a_wide_list_is_a_tree_and_edits_share() {
        let rope = Children::from_vec((0..30_000).map(text).collect());
        assert!(rope.height() >= 2);
        let edited = rope.splice(15_000..15_001, vec![text(1)]).unwrap();
        assert_eq!(edited.get(15_000).unwrap().as_text(), Some("1"));
        assert_eq!(edited.len(), 30_000);
        assert_eq!(rope.splice(0..30_001, vec![]).map(|c| c.len()), None);
        assert!(rope.splice(0..30_000, vec![]).unwrap().is_empty());
    }
}
