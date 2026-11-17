//! Owns: what a document's ids have been beside what they are now -- docx-agent's `Ledger`
//! and `IdPool` (`docx_agent/edit/ids.py`, round 37): every `ST_LongHexNumber` value any
//! state had or generation gave (the pool), each block id and drawing number gone with the
//! version that removed it, the renames (aliases); and the deterministic sequences new
//! values are drawn from (a seed's SHA-256 stream), so the same edit writes the same bytes.
//!
//! Must not: read a package (it is given the parts' roots), stamp, or know a tool's reply.
//! Each pool keeps the sequences it has drawn from (a document's cache, never a global one).

use crate::ids::sha256::sha256;
use crate::ids::{hex_value, W, W14, WP, WP14};
use ooxml_xml::{Element, Node};
use std::collections::{BTreeMap, HashMap, HashSet};

/// How far [`IdPool::generate`] looks along a seed's sequence past the last value in use
/// before it takes new ones (docx-agent's `REACH`).
pub const REACH: usize = 4096;

/// A seed's sequence value at `counter`: the first four bytes of SHA-256 of
/// `"{seed}\0{counter}"`, big-endian, top bit cleared.
#[must_use]
pub fn sequence_value(seed: &str, counter: usize) -> u32 {
    let digest = sha256(format!("{seed}\0{counter}").as_bytes());
    let [a, b, c, d, ..] = digest;
    u32::from_be_bytes([a, b, c, d]) & 0x7FFF_FFFF
}

/// A document's ids, in use and retired: values, and where along each seed's sequence
/// generation got to (every value before it is in the set).
#[derive(Debug, Clone, Default)]
pub struct IdPool {
    values: HashSet<u32>,
    cursors: BTreeMap<String, usize>,
    sequences: HashMap<String, Vec<u32>>,
}

impl IdPool {
    /// A pool of `values`.
    #[must_use]
    pub fn of(values: HashSet<u32>) -> IdPool {
        IdPool {
            values,
            ..IdPool::default()
        }
    }

    /// Whether `value` is in the pool.
    #[must_use]
    pub fn contains(&self, value: u32) -> bool {
        self.values.contains(&value)
    }

    /// Every value.
    #[must_use]
    pub fn values(&self) -> &HashSet<u32> {
        &self.values
    }

    /// Add values.
    pub fn extend(&mut self, values: impl IntoIterator<Item = u32>) {
        self.values.extend(values);
    }

    /// Where each seed's generation got to.
    #[must_use]
    pub fn cursors(&self) -> &BTreeMap<String, usize> {
        &self.cursors
    }

    /// Set where `seed`'s generation got to.
    pub fn set_cursor(&mut self, seed: &str, k: usize) {
        self.cursors.insert(seed.to_owned(), k);
    }

    /// The first `length` values of `seed`'s sequence (more where they were drawn already).
    pub fn sequence(&mut self, seed: &str, length: usize) -> &[u32] {
        let found = self.sequences.entry(seed.to_owned()).or_default();
        if found.len() < length {
            let target = length.max(2 * found.len());
            let start = found.len();
            found.extend((start..target).map(|k| sequence_value(seed, k)));
        }
        found
    }

    /// Where along `seed`'s sequence new values start: after the last one the pool holds
    /// with no later one within [`REACH`].
    fn after_last(&mut self, seed: &str) -> usize {
        let mut last: Option<usize> = None;
        let mut k = 0_usize;
        loop {
            let reach = match last {
                Some(l) => k - l,
                None => k + 1,
            };
            if reach > REACH {
                break;
            }
            let value = self.sequence(seed, k + 1).get(k).copied();
            if value.is_some_and(|v| self.values.contains(&v)) {
                last = Some(k);
            }
            k += 1;
        }
        last.map_or(0, |l| l + 1)
    }

    /// `count` new values, deterministic in `seed` and the pool: the next members of the
    /// seed's sequence the pool lacks, never 0, upper-case eight-digit hex as Word writes
    /// them; the pool gains them (docx-agent's `generate`).
    pub fn generate(&mut self, seed: &str, count: usize) -> Vec<String> {
        let mut k = match self.cursors.get(seed) {
            Some(k) => *k,
            None => self.after_last(seed),
        };
        let mut out = Vec::with_capacity(count);
        while out.len() < count {
            let value = self.sequence(seed, k + 1).get(k).copied().unwrap_or(0);
            k += 1;
            if value == 0 || self.values.contains(&value) {
                continue;
            }
            self.values.insert(value);
            out.push(format!("{value:08X}"));
        }
        self.cursors.insert(seed.to_owned(), k);
        out
    }
}

/// What a document's ids have been this session, beside what they are now.
#[derive(Debug, Clone, Default)]
pub struct Ledger {
    /// Every value any state had and every one generated (`None` until a state is read).
    pub pool: Option<IdPool>,
    /// The version last read.
    pub version: Option<i64>,
    /// The block id values of the state last read.
    pub blocks: HashSet<u32>,
    /// Each block id value no longer in the document, with the version that removed it.
    pub gone: HashMap<u32, i64>,
    /// The drawing numbers of the state last read.
    pub drawings: HashSet<u64>,
    /// Each drawing number gone, with the version that removed it.
    pub drawings_gone: HashMap<u64, i64>,
    /// The highest drawing number any state had.
    pub highest_drawing: u64,
    /// Block id value -> the id of the one block that replaced it.
    pub replaced: HashMap<u32, String>,
    /// Changes made before the document was saved and opened again, by version: who made
    /// each, in words (`None`: not known).
    pub earlier: HashMap<i64, Option<String>>,
    /// Renames: an old id -> the id it names now.
    pub aliases: BTreeMap<String, String>,
}

impl Ledger {
    /// Read the state whose parts' roots are `roots`, as version `version`: what is gone
    /// since the last state read went in `version`; what is back is no longer gone.
    pub fn note(&mut self, roots: &[&Element], version: i64) {
        if self.version == Some(version) && self.pool.is_some() {
            return;
        }
        let (blocks, drawings) = block_and_drawing_ids(roots);
        match &mut self.pool {
            None => self.pool = Some(IdPool::of(used_long_hex(roots))),
            Some(_) => {
                for value in self.blocks.difference(&blocks) {
                    self.gone.insert(*value, version);
                }
                for value in self.drawings.difference(&drawings) {
                    self.drawings_gone.insert(*value, version);
                }
            }
        }
        for value in &blocks {
            self.gone.remove(value);
        }
        for value in &drawings {
            self.drawings_gone.remove(value);
        }
        if let Some(pool) = &mut self.pool {
            pool.extend(blocks.iter().copied());
        }
        self.highest_drawing = self.highest_drawing.max(drawings.iter().copied().max().unwrap_or(0));
        self.blocks = blocks;
        self.drawings = drawings;
        self.version = Some(version);
    }

    /// The pool, read from `roots` first where no state was read yet.
    pub fn pool(&mut self, roots: &[&Element], version: i64) -> &mut IdPool {
        if self.pool.is_none() {
            self.note(roots, version);
        }
        self.pool.get_or_insert_with(IdPool::default)
    }

    /// The id `id` names now, following renames while their target exists (`exists`).
    #[must_use]
    pub fn follow(&self, id: &str, exists: impl Fn(&str) -> bool) -> String {
        let mut seen = HashSet::new();
        let mut target = id;
        while let Some(next) = self.aliases.get(target) {
            if !seen.insert(target) {
                break;
            }
            target = next;
        }
        if target != id && exists(target) {
            target.to_owned()
        } else {
            id.to_owned()
        }
    }
}

/// Every paraId and textId, and every drawing's anchorId and editId, written anywhere in
/// `roots` (fallbacks included).
#[must_use]
pub fn used_long_hex(roots: &[&Element]) -> HashSet<u32> {
    let mut used = HashSet::new();
    for root in roots {
        visit(root, &mut |e| {
            let names: &[(&str, &str)] = if e.name().is(Some(W), "p") || e.name().is(Some(W), "tr") {
                &[(W14, "paraId"), (W14, "textId")]
            } else if e.name().is(Some(WP), "inline") || e.name().is(Some(WP), "anchor") {
                &[(WP14, "anchorId"), (WP14, "editId")]
            } else {
                return;
            };
            for (namespace, local) in names {
                if let Some(value) = e.attribute(Some(namespace), local).and_then(hex_value) {
                    used.insert(value);
                }
            }
        });
    }
    used
}

/// Every paragraph's and row's paraId (as a value) and every drawing number in `roots`.
#[must_use]
pub fn block_and_drawing_ids(roots: &[&Element]) -> (HashSet<u32>, HashSet<u64>) {
    let mut blocks = HashSet::new();
    let mut drawings = HashSet::new();
    for root in roots {
        visit(root, &mut |e| {
            if e.name().is(Some(WP), "docPr") {
                if let Some(n) = e.attribute(None, "id").and_then(digits) {
                    drawings.insert(n);
                }
            } else if e.name().is(Some(W), "p") || e.name().is(Some(W), "tr") {
                if let Some(v) = e.attribute(Some(W14), "paraId").and_then(hex_value) {
                    blocks.insert(v);
                }
            }
        });
    }
    (blocks, drawings)
}

/// Every drawing number (`wp:docPr/@id`) written in `roots`.
#[must_use]
pub fn drawing_numbers(roots: &[&Element]) -> HashSet<u64> {
    block_and_drawing_ids(roots).1
}

/// A number written in ASCII digits only (Python's `isdigit` then `int`, for what Word writes).
fn digits(text: &str) -> Option<u64> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// Call `f` on every element below `root`, itself included, in document order.
fn visit(root: &Element, f: &mut impl FnMut(&Element)) {
    f(root);
    let mut stack = vec![root.children().iter()];
    while let Some(top) = stack.last_mut() {
        match top.next() {
            Some(Node::Element(e)) => {
                f(e);
                stack.push(e.children().iter());
            }
            Some(_) => {}
            None => {
                stack.pop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_skips_used_values_and_continues_after_the_last() {
        let first = sequence_value("word/document.xml", 0);
        let second = sequence_value("word/document.xml", 1);
        let mut pool = IdPool::default();
        let made = pool.generate("word/document.xml", 2);
        assert_eq!(made, vec![format!("{first:08X}"), format!("{second:08X}")]);
        // A document that has the second value but not the first: generation goes on after it.
        let mut pool = IdPool::of([second].into_iter().collect());
        let made = pool.generate("word/document.xml", 1);
        assert_eq!(made, vec![format!("{:08X}", sequence_value("word/document.xml", 2))]);
    }
}
