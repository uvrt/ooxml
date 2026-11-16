//! Owns: [`MonoidTree`], a persistent sequence (a height-balanced tree of leaf chunks) whose
//! nodes cache their length and their items' aggregate, so that a change, an insert, a
//! removal and a prefix or range aggregate each cost O(log n), and a clone costs O(1)
//! (a derived snapshot is published by cloning).
//!
//! Must not: know what its items are beyond [`Measured`]; mutate a node another clone can
//! see (every change copies its path).

use crate::{DeriveError, Measured, Monoid};
use std::sync::Arc;

/// The most items a leaf holds; a full leaf splits in two.
const CHUNK: usize = 32;

/// A persistent sequence with its aggregate kept per subtree.
pub struct MonoidTree<T: Measured> {
    root: Option<Arc<Node<T>>>,
}

/// A shared node.
type Link<T> = Arc<Node<T>>;

struct Node<T: Measured> {
    len: usize,
    height: u8,
    summary: T::Summary,
    kind: Kind<T>,
}

enum Kind<T: Measured> {
    Leaf(Vec<T>),
    Branch(Arc<Node<T>>, Arc<Node<T>>),
}

impl<T: Measured> Clone for MonoidTree<T> {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
        }
    }
}

impl<T: Measured> Default for MonoidTree<T> {
    fn default() -> Self {
        Self { root: None }
    }
}

impl<T: Measured> std::fmt::Debug for MonoidTree<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonoidTree")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl<T: Measured> FromIterator<T> for MonoidTree<T> {
    /// A balanced tree of full leaves, built in O(n).
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
        let mut leaves = Vec::new();
        let mut chunk = Vec::with_capacity(CHUNK);
        for item in items {
            chunk.push(item);
            if chunk.len() == CHUNK {
                leaves.push(leaf(std::mem::replace(&mut chunk, Vec::with_capacity(CHUNK))));
            }
        }
        if !chunk.is_empty() {
            leaves.push(leaf(chunk));
        }
        Self { root: build(&leaves) }
    }
}

impl<T: Measured> MonoidTree<T> {
    /// The empty sequence.
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of items.
    pub fn len(&self) -> usize {
        self.root.as_ref().map_or(0, |node| node.len)
    }

    /// No items.
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// The aggregate of every item.
    pub fn summary(&self) -> T::Summary {
        self.root
            .as_ref()
            .map_or_else(T::Summary::empty, |node| node.summary.clone())
    }

    /// The item at `at`.
    pub fn get(&self, at: usize) -> Option<&T> {
        let mut node = self.root.as_deref()?;
        let mut at = at;
        loop {
            match &node.kind {
                Kind::Leaf(items) => return items.get(at),
                Kind::Branch(left, right) if at < left.len => node = left,
                Kind::Branch(left, right) => {
                    at -= left.len;
                    node = right;
                }
            }
        }
    }

    /// The aggregate of the items before `at` (`at` may be the length).
    pub fn prefix(&self, at: usize) -> Result<T::Summary, DeriveError> {
        self.prefix_by(at, T::Summary::clone)
    }

    /// The aggregate of the items before `at` seen through `view`, which must be a monoid
    /// homomorphism (`view(a.combine(b))` equals `view(a).combine(view(b))`, and `view` of
    /// the identity is the identity). A query that needs a small part of a large aggregate
    /// -- the counters of the few lists in a range, out of every list of the document --
    /// pays for that part only.
    pub fn prefix_by<S: Monoid>(&self, at: usize, view: impl Fn(&T::Summary) -> S) -> Result<S, DeriveError> {
        self.check(at, self.len())?;
        let mut acc = S::empty();
        let mut node = self.root.as_deref();
        let mut at = at;
        while let Some(current) = node {
            match &current.kind {
                Kind::Leaf(items) => {
                    for item in items.iter().take(at) {
                        acc = acc.combine(&view(&item.measure()));
                    }
                    node = None;
                }
                Kind::Branch(left, _) if at <= left.len => node = Some(left),
                Kind::Branch(left, right) => {
                    acc = acc.combine(&view(&left.summary));
                    at -= left.len;
                    node = Some(right);
                }
            }
        }
        Ok(acc)
    }

    /// The aggregate of the items in `from..to`.
    pub fn range(&self, from: usize, to: usize) -> Result<T::Summary, DeriveError> {
        self.check(to, self.len())?;
        self.check(from, to)?;
        let mut acc = T::Summary::empty();
        for item in self.iter_from(from)?.take(to - from) {
            acc = acc.combine(&item.measure());
        }
        Ok(acc)
    }

    /// The items in order.
    pub fn iter(&self) -> Iter<'_, T> {
        Iter::new(self.root.as_deref(), 0)
    }

    /// The items from `at` on.
    pub fn iter_from(&self, at: usize) -> Result<Iter<'_, T>, DeriveError> {
        self.check(at, self.len())?;
        Ok(Iter::new(self.root.as_deref(), at))
    }

    /// Replace the item at `at`.
    pub fn set(&mut self, at: usize, item: T) -> Result<(), DeriveError> {
        let root = self
            .root
            .as_ref()
            .filter(|_| at < self.len())
            .ok_or(DeriveError::OutOfRange { at, len: self.len() })?;
        self.root = Some(set(root, at, item));
        Ok(())
    }

    /// Insert `item` before the item at `at` (at the end when `at` is the length).
    pub fn insert(&mut self, at: usize, item: T) -> Result<(), DeriveError> {
        self.check(at, self.len())?;
        self.root = Some(match &self.root {
            None => leaf(vec![item]),
            Some(root) => insert(root, at, item),
        });
        Ok(())
    }

    /// Remove and return the item at `at`.
    pub fn remove(&mut self, at: usize) -> Result<T, DeriveError> {
        let root = self
            .root
            .as_ref()
            .filter(|_| at < self.len())
            .ok_or(DeriveError::OutOfRange { at, len: self.len() })?;
        let (rest, item) = remove(root, at);
        self.root = rest;
        item.ok_or(DeriveError::OutOfRange { at, len: self.len() })
    }

    fn check(&self, at: usize, limit: usize) -> Result<(), DeriveError> {
        if at > limit {
            return Err(DeriveError::OutOfRange { at, len: self.len() });
        }
        Ok(())
    }

    /// Whether every node's height, length and balance are as they should be (tests).
    #[doc(hidden)]
    pub fn is_well_formed(&self) -> bool {
        self.root.as_deref().is_none_or(well_formed)
    }
}

fn well_formed<T: Measured>(node: &Node<T>) -> bool {
    match &node.kind {
        Kind::Leaf(items) => node.height == 0 && node.len == items.len() && !items.is_empty() && items.len() <= CHUNK,
        Kind::Branch(left, right) => {
            node.len == left.len + right.len
                && node.height == 1 + left.height.max(right.height)
                && left.height.abs_diff(right.height) <= 1
                && well_formed(left)
                && well_formed(right)
        }
    }
}

fn leaf<T: Measured>(items: Vec<T>) -> Arc<Node<T>> {
    let mut summary = T::Summary::empty();
    for item in &items {
        summary = summary.combine(&item.measure());
    }
    Arc::new(Node {
        len: items.len(),
        height: 0,
        summary,
        kind: Kind::Leaf(items),
    })
}

/// A branch of `left` and `right` as they are (the caller keeps the balance).
fn branch<T: Measured>(left: Arc<Node<T>>, right: Arc<Node<T>>) -> Arc<Node<T>> {
    Arc::new(Node {
        len: left.len + right.len,
        height: 1 + left.height.max(right.height),
        summary: left.summary.combine(&right.summary),
        kind: Kind::Branch(left, right),
    })
}

fn children<T: Measured>(node: &Link<T>) -> Option<(&Link<T>, &Link<T>)> {
    match &node.kind {
        Kind::Branch(left, right) => Some((left, right)),
        Kind::Leaf(_) => None,
    }
}

/// `left` then `right`, balanced (heights of siblings differ by at most one). Two small
/// leaves become one.
fn join<T: Measured>(left: Arc<Node<T>>, right: Arc<Node<T>>) -> Arc<Node<T>> {
    if let (Kind::Leaf(a), Kind::Leaf(b)) = (&left.kind, &right.kind) {
        if a.len() + b.len() <= CHUNK {
            return leaf(a.iter().chain(b.iter()).cloned().collect());
        }
    }
    if left.height > right.height + 1 {
        if let Some((ll, lr)) = children(&left) {
            let joined = join(Arc::clone(lr), right);
            return rebalance(Arc::clone(ll), joined);
        }
    } else if right.height > left.height + 1 {
        if let Some((rl, rr)) = children(&right) {
            let joined = join(left, Arc::clone(rl));
            return rebalance(joined, Arc::clone(rr));
        }
    }
    branch(left, right)
}

/// A branch of two balanced subtrees whose heights differ by at most two.
fn rebalance<T: Measured>(left: Arc<Node<T>>, right: Arc<Node<T>>) -> Arc<Node<T>> {
    if right.height > left.height + 1 {
        if let Some((rl, rr)) = children(&right) {
            if rr.height >= rl.height {
                return branch(branch(left, Arc::clone(rl)), Arc::clone(rr));
            }
            if let Some((rll, rlr)) = children(rl) {
                return branch(branch(left, Arc::clone(rll)), branch(Arc::clone(rlr), Arc::clone(rr)));
            }
        }
    } else if left.height > right.height + 1 {
        if let Some((ll, lr)) = children(&left) {
            if ll.height >= lr.height {
                return branch(Arc::clone(ll), branch(Arc::clone(lr), right));
            }
            if let Some((lrl, lrr)) = children(lr) {
                return branch(branch(Arc::clone(ll), Arc::clone(lrl)), branch(Arc::clone(lrr), right));
            }
        }
    }
    branch(left, right)
}

/// A balanced tree over `leaves`, in order.
fn build<T: Measured>(leaves: &[Arc<Node<T>>]) -> Option<Arc<Node<T>>> {
    match leaves {
        [] => None,
        [one] => Some(Arc::clone(one)),
        _ => {
            let (left, right) = leaves.split_at(leaves.len() / 2);
            Some(branch(build(left)?, build(right)?))
        }
    }
}

fn set<T: Measured>(node: &Arc<Node<T>>, at: usize, item: T) -> Arc<Node<T>> {
    match &node.kind {
        Kind::Leaf(items) => {
            let mut items = items.clone();
            if let Some(slot) = items.get_mut(at) {
                *slot = item;
            }
            leaf(items)
        }
        Kind::Branch(left, right) if at < left.len => branch(set(left, at, item), Arc::clone(right)),
        Kind::Branch(left, right) => branch(Arc::clone(left), set(right, at - left.len, item)),
    }
}

fn insert<T: Measured>(node: &Arc<Node<T>>, at: usize, item: T) -> Arc<Node<T>> {
    match &node.kind {
        Kind::Leaf(items) => {
            let mut items = items.clone();
            items.insert(at.min(items.len()), item);
            if items.len() <= CHUNK {
                return leaf(items);
            }
            let back = items.split_off(items.len() / 2);
            branch(leaf(items), leaf(back))
        }
        Kind::Branch(left, right) if at <= left.len => join(insert(left, at, item), Arc::clone(right)),
        Kind::Branch(left, right) => join(Arc::clone(left), insert(right, at - left.len, item)),
    }
}

/// `node` without the item at `at` (`None` when nothing is left), and the item.
fn remove<T: Measured>(node: &Arc<Node<T>>, at: usize) -> (Option<Arc<Node<T>>>, Option<T>) {
    match &node.kind {
        Kind::Leaf(items) => {
            let mut items = items.clone();
            let item = (at < items.len()).then(|| items.remove(at));
            ((!items.is_empty()).then(|| leaf(items)), item)
        }
        Kind::Branch(left, right) => {
            let (rest, item, other, rest_is_left) = if at < left.len {
                let (rest, item) = remove(left, at);
                (rest, item, right, true)
            } else {
                let (rest, item) = remove(right, at - left.len);
                (rest, item, left, false)
            };
            let joined = match rest {
                None => Arc::clone(other),
                Some(rest) if rest_is_left => join(rest, Arc::clone(other)),
                Some(rest) => join(Arc::clone(other), rest),
            };
            (Some(joined), item)
        }
    }
}

/// The items of a [`MonoidTree`] in order, from a position.
pub struct Iter<'a, T: Measured> {
    /// Right subtrees still to visit, innermost last.
    pending: Vec<&'a Node<T>>,
    items: std::slice::Iter<'a, T>,
}

impl<'a, T: Measured> Iter<'a, T> {
    fn new(root: Option<&'a Node<T>>, at: usize) -> Self {
        let mut pending = Vec::new();
        let mut node = root;
        let mut at = at;
        let mut items = [].iter();
        while let Some(current) = node {
            match &current.kind {
                Kind::Leaf(leaf_items) => {
                    items = leaf_items.get(at..).unwrap_or_default().iter();
                    node = None;
                }
                Kind::Branch(left, right) if at < left.len => {
                    pending.push(&**right);
                    node = Some(left);
                }
                Kind::Branch(left, right) => {
                    at -= left.len;
                    node = Some(right);
                }
            }
        }
        Self { pending, items }
    }
}

impl<'a, T: Measured> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        loop {
            if let Some(item) = self.items.next() {
                return Some(item);
            }
            let mut node = self.pending.pop()?;
            loop {
                match &node.kind {
                    Kind::Leaf(items) => {
                        self.items = items.iter();
                        break;
                    }
                    Kind::Branch(left, right) => {
                        self.pending.push(right);
                        node = left;
                    }
                }
            }
        }
    }
}
