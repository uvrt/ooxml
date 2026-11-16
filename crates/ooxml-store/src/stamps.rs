//! Owns: [`Stamps`], per key the number of the version that last wrote it, as a persistent
//! B-tree (sorted leaves of at most [`FANOUT`] entries under branches of at most
//! [`FANOUT`] children, shared between versions): stamping a few keys copies their paths,
//! a few hundred bytes, so a version costs only what it changed, and the copy is cheap
//! enough for the commit's critical section.
//!
//! Must not: know what a key is, or remove a key (a stamp is only ever raised).
//!
//! (A persistent map crate was considered: imbl, the approved candidate, is MPL-2.0, which
//! the licence policy of design section 2.7 does not allow for a dependency.)

use crate::{KeySet, Seq};
use std::sync::Arc;

/// Most entries in a leaf and children in a branch; a node that grows past it splits.
const FANOUT: usize = 32;

enum Node<K> {
    /// Keys in order with their stamps.
    Leaf(Vec<(K, Seq)>),
    /// Children in order, each with its first key.
    Branch(Vec<(K, Arc<Node<K>>)>),
}

/// Per key, the version that last wrote it.
pub(crate) struct Stamps<K> {
    root: Option<Arc<Node<K>>>,
    /// The last version that wrote every key.
    everything: Seq,
}

impl<K> Clone for Stamps<K> {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            everything: self.everything,
        }
    }
}

impl<K> Default for Stamps<K> {
    fn default() -> Self {
        Self {
            root: None,
            everything: Seq(0),
        }
    }
}

impl<K> std::fmt::Debug for Stamps<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stamps")
            .field("everything", &self.everything)
            .finish_non_exhaustive()
    }
}

impl<K: Clone + Ord> Stamps<K> {
    /// The last version that wrote `key`; `Seq(0)` when none did.
    pub(crate) fn of(&self, key: &K) -> Seq {
        let mut node = self.root.as_deref();
        while let Some(current) = node {
            match current {
                Node::Leaf(entries) => {
                    let found = entries
                        .binary_search_by(|(k, _)| k.cmp(key))
                        .ok()
                        .and_then(|at| entries.get(at));
                    return found.map_or(Seq(0), |(_, seq)| *seq).max(self.everything);
                }
                Node::Branch(children) => {
                    let at = children.partition_point(|(first, _)| first <= key).saturating_sub(1);
                    node = children.get(at).map(|(_, child)| &**child);
                }
            }
        }
        self.everything
    }

    /// The last version that wrote every key at once.
    pub(crate) fn everything(&self) -> Seq {
        self.everything
    }

    /// Stamp `writes` with `seq`.
    pub(crate) fn stamp(&mut self, writes: &KeySet<K>, seq: Seq) {
        match writes.keys() {
            None => self.everything = seq,
            Some(keys) => {
                for key in keys {
                    self.root = Some(match self.root.take() {
                        None => Arc::new(Node::Leaf(vec![(key.clone(), seq)])),
                        Some(root) => match insert(&root, key, seq) {
                            (node, None) => node,
                            (left, Some(right)) => {
                                let first = |node: &Arc<Node<K>>| first_key(node).cloned();
                                match (first(&left), first(&right)) {
                                    (Some(a), Some(b)) => Arc::new(Node::Branch(vec![(a, left), (b, right)])),
                                    // A split never leaves a side empty.
                                    _ => left,
                                }
                            }
                        },
                    });
                }
            }
        }
    }
}

fn first_key<K>(node: &Node<K>) -> Option<&K> {
    match node {
        Node::Leaf(entries) => entries.first().map(|(k, _)| k),
        Node::Branch(children) => children.first().map(|(k, _)| k),
    }
}

/// `node` with `key` stamped, and the right half when it split.
fn insert<K: Clone + Ord>(node: &Node<K>, key: &K, seq: Seq) -> (Arc<Node<K>>, Option<Arc<Node<K>>>) {
    match node {
        Node::Leaf(entries) => {
            let mut entries = entries.clone();
            match entries.binary_search_by(|(k, _)| k.cmp(key)) {
                Ok(at) => {
                    if let Some(slot) = entries.get_mut(at) {
                        slot.1 = seq;
                    }
                }
                Err(at) => entries.insert(at, (key.clone(), seq)),
            }
            split(entries, Node::Leaf)
        }
        Node::Branch(children) => {
            let at = children.partition_point(|(first, _)| first <= key).saturating_sub(1);
            let mut children: Vec<(K, Arc<Node<K>>)> = children.clone();
            let Some((first, child)) = children.get_mut(at) else {
                return (Arc::new(Node::Branch(children)), None);
            };
            let (changed, right) = insert(child, key, seq);
            if key < &*first {
                *first = key.clone();
            }
            *child = changed;
            if let Some(right) = right {
                if let Some(right_first) = first_key(&right).cloned() {
                    children.insert(at + 1, (right_first, right));
                }
            }
            split(children, Node::Branch)
        }
    }
}

/// A node of `items`, split in two halves when it has more than [`FANOUT`].
fn split<K, T>(mut items: Vec<T>, make: fn(Vec<T>) -> Node<K>) -> (Arc<Node<K>>, Option<Arc<Node<K>>>) {
    if items.len() <= FANOUT {
        return (Arc::new(make(items)), None);
    }
    let right = items.split_off(items.len() / 2);
    (Arc::new(make(items)), Some(Arc::new(make(right))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn stamps_equal_a_map_and_versions_stay_as_they_were() {
        let mut stamps = Stamps::default();
        let mut oracle: BTreeMap<u32, Seq> = BTreeMap::new();
        let mut kept = Vec::new();
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        for step in 1..=5_000u64 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let keys: Vec<u32> = (0..1 + x % 4).map(|i| ((x >> (8 * i)) % 3_000) as u32).collect();
            stamps.stamp(&KeySet::of(keys.iter().copied()), Seq(step));
            for key in keys {
                oracle.insert(key, Seq(step));
            }
            if step % 500 == 0 {
                kept.push((stamps.clone(), oracle.clone()));
            }
        }
        for key in 0..3_100u32 {
            assert_eq!(
                stamps.of(&key),
                oracle.get(&key).copied().unwrap_or(Seq(0)),
                "key {key}"
            );
        }
        for (old, map) in &kept {
            for key in (0..3_100u32).step_by(7) {
                assert_eq!(old.of(&key), map.get(&key).copied().unwrap_or(Seq(0)));
            }
        }
        stamps.stamp(&KeySet::Everything, Seq(9_000));
        assert_eq!(stamps.of(&1), Seq(9_000));
        assert_eq!(stamps.everything(), Seq(9_000));
        assert_eq!(kept[0].0.everything(), Seq(0));
    }
}
