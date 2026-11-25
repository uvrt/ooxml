//! Owns: which chunks of a new version travel to a host that holds the previous version's
//! leaves (part C: a leaf is reused whenever signature and size match).
//!
//! Must not: look inside a chunk: a chunk over 1 MiB is reused or sent whole, through its
//! deterministic parent leaf (its subchunks' random signatures never match).

use super::{Chunking, Leaf};
use std::collections::{HashMap, HashSet};

/// How one chunk of the new version reaches the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reuse {
    /// The host holds its leaf already (a previous version's leaf of the same signature and
    /// size).
    Held,
    /// The same leaf as an earlier chunk of this version (by index), which travels.
    Earlier(usize),
    /// It travels.
    New,
}

/// A new version's chunks against the previous version's leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delta {
    reuse: Vec<Reuse>,
    new_bytes: u64,
}

impl Delta {
    pub(super) fn new(chunking: &Chunking, previous: &[Leaf]) -> Self {
        let held: HashSet<&Leaf> = previous.iter().collect();
        let mut sent: HashMap<Leaf, usize> = HashMap::new();
        let mut new_bytes = 0u64;
        let reuse = chunking
            .chunks()
            .iter()
            .enumerate()
            .map(|(index, chunk)| {
                let leaf = chunk.leaf();
                if held.contains(&leaf) {
                    return Reuse::Held;
                }
                if let Some(&first) = sent.get(&leaf) {
                    return Reuse::Earlier(first);
                }
                sent.insert(leaf, index);
                new_bytes += leaf.size;
                Reuse::New
            })
            .collect();
        Delta { reuse, new_bytes }
    }

    /// Each chunk's reuse, in the chunking's order.
    pub fn reuse(&self) -> &[Reuse] {
        &self.reuse
    }

    /// The indexes of the chunks that travel, in file order.
    pub fn new_chunks(&self) -> Vec<usize> {
        self.indexes(|r| r == Reuse::New)
    }

    /// The indexes of the chunks the host holds already, in file order.
    pub fn held_chunks(&self) -> Vec<usize> {
        self.indexes(|r| r == Reuse::Held)
    }

    /// The bytes that travel: the sizes of the new chunks.
    pub fn new_bytes(&self) -> u64 {
        self.new_bytes
    }

    fn indexes(&self, keep: impl Fn(Reuse) -> bool) -> Vec<usize> {
        self.reuse
            .iter()
            .enumerate()
            .filter(|&(_, &r)| keep(r))
            .map(|(i, _)| i)
            .collect()
    }
}
