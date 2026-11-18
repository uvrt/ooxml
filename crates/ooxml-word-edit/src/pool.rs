//! Owns: a version's ids as edits draw them -- docx-agent's pool and ledger
//! (`edit/ids.py`, `generate`) as a persistent value: every `ST_LongHexNumber` value any
//! state had or a draw gave, where each seed's sequence got to, and the renames
//! ([`Ids`]); new values drawn deterministically from a seed's SHA-256 sequence, so the
//! same edit writes the same bytes ([`Ids::draw`]); the sequences themselves, computed once
//! per document ([`Sequences`]).
//!
//! A draw reads and writes its seed's key (`Key::Seq`): two transactions drawing from one
//! seed conflict, and the later one draws again on the newer version, so values are those
//! a serial run gives (design section 5's ledger, with the draw order fixed by the commit
//! order).
//!
//! Must not: stamp a part (the sink does), read a package, or hold global state.

use ooxml_word_model::ids::{sequence_value, REACH};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

/// A seed's sequence values, computed as far as they were needed; one per document.
#[derive(Debug, Default)]
pub struct Sequences {
    found: Mutex<HashMap<String, Arc<Vec<u32>>>>,
}

impl Sequences {
    /// The first `length` (or more) values of `seed`'s sequence.
    fn get(&self, seed: &str, length: usize) -> Arc<Vec<u32>> {
        let mut found = self.found.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = found.entry(seed.to_owned()).or_default();
        if entry.len() < length {
            let target = length.max(2 * entry.len());
            let mut values = Vec::with_capacity(target);
            values.extend_from_slice(entry);
            let start = values.len();
            values.extend((start..target).map(|k| sequence_value(seed, k)));
            *entry = Arc::new(values);
        }
        Arc::clone(entry)
    }
}

/// What a draw adds to a version's ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdsChange {
    /// Values now in the pool.
    pub values: Vec<u32>,
    /// Seeds and where their generation got to.
    pub cursors: Vec<(String, usize)>,
    /// Renames: an old id and the id it names now.
    pub aliases: Vec<(String, String)>,
    /// Ids whose aliases go, after the renames (blocks an edit removed: docx-agent's
    /// `_aliases` filter in `delete_block`).
    pub forget: Vec<String>,
    /// Ids no alias may name any more, after the renames (paragraphs a range edit joined
    /// away: docx-agent's `_aliases` filter in its range edits).
    pub forget_targets: Vec<String>,
}

impl IdsChange {
    /// Whether it adds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
            && self.cursors.is_empty()
            && self.aliases.is_empty()
            && self.forget.is_empty()
            && self.forget_targets.is_empty()
    }
}

/// A version's ids: the pool, the sequences' cursors and the renames. Cloning shares.
#[derive(Debug, Clone, Default)]
pub struct Ids {
    base: Arc<HashSet<u32>>,
    added: Arc<BTreeSet<u32>>,
    cursors: Arc<BTreeMap<String, usize>>,
    aliases: Arc<BTreeMap<String, String>>,
    /// The first annotation id of the edit being applied (one above the largest in use
    /// when it began numbering): its revision records' ids count from here.
    annotations: Option<i64>,
}

impl Ids {
    /// The ids of a document as opened: `values`, every one its parts hold.
    #[must_use]
    pub fn opened(values: HashSet<u32>) -> Ids {
        Ids {
            base: Arc::new(values),
            ..Ids::default()
        }
    }

    /// Whether `value` is in the pool.
    #[must_use]
    pub fn contains(&self, value: u32) -> bool {
        self.base.contains(&value) || self.added.contains(&value)
    }

    /// The renames: an old id and the id it names now.
    #[must_use]
    pub fn aliases(&self) -> &BTreeMap<String, String> {
        &self.aliases
    }

    /// The id `id` names now, following renames.
    #[must_use]
    pub fn follow(&self, id: &str) -> String {
        let mut seen = HashSet::new();
        let mut target = id;
        while let Some(next) = self.aliases.get(target) {
            if !seen.insert(target) {
                break;
            }
            target = next;
        }
        target.to_owned()
    }

    /// The first annotation id of the edit being applied.
    #[must_use]
    pub fn annotations(&self) -> Option<i64> {
        self.annotations
    }

    /// Start numbering an edit's revision records from `first`.
    pub(crate) fn number_from(&mut self, first: i64) {
        self.annotations = Some(first);
    }

    /// Add what `change` adds.
    pub fn apply(&mut self, change: &IdsChange) {
        if !change.values.is_empty() {
            Arc::make_mut(&mut self.added).extend(change.values.iter().copied());
        }
        if !change.cursors.is_empty() {
            let cursors = Arc::make_mut(&mut self.cursors);
            for (seed, k) in &change.cursors {
                cursors.insert(seed.clone(), *k);
            }
        }
        if !change.aliases.is_empty() {
            let aliases = Arc::make_mut(&mut self.aliases);
            for (old, new) in &change.aliases {
                aliases.insert(old.clone(), new.clone());
                // an alias that pointed at the old id follows it to the new one
                for value in aliases.values_mut() {
                    if value == old {
                        value.clone_from(new);
                    }
                }
            }
        }
        if self.aliases.values().any(|v| change.forget_targets.contains(v)) {
            Arc::make_mut(&mut self.aliases).retain(|_, v| !change.forget_targets.contains(v));
        }
        if change.forget.iter().any(|id| self.aliases.contains_key(id)) {
            let aliases = Arc::make_mut(&mut self.aliases);
            for id in &change.forget {
                aliases.remove(id);
            }
        }
    }

    /// Where `seed`'s new values start: after the last one the pool holds with no later one
    /// within [`REACH`].
    fn after_last(&self, seed: &str, sequences: &Sequences) -> usize {
        let mut last: Option<usize> = None;
        let mut k = 0usize;
        let mut values = sequences.get(seed, REACH + 1);
        loop {
            let reach = last.map_or(k + 1, |l| k - l);
            if reach > REACH {
                break;
            }
            if values.len() <= k {
                values = sequences.get(seed, 2 * k + 16);
            }
            if values.get(k).is_some_and(|v| self.contains(*v)) {
                last = Some(k);
            }
            k += 1;
        }
        last.map_or(0, |l| l + 1)
    }

    /// `count` new values of `seed` (docx-agent's `generate`): the next members of the
    /// seed's sequence the pool lacks, never 0, as eight upper-case hex digits; and the
    /// change that adds them and the cursor to the pool.
    #[must_use]
    pub fn draw(&self, seed: &str, count: usize, sequences: &Sequences) -> (Vec<String>, IdsChange) {
        let mut k = match self.cursors.get(seed) {
            Some(k) => *k,
            None => self.after_last(seed, sequences),
        };
        let mut values = sequences.get(seed, k + 2 * count + 16);
        let mut out = Vec::with_capacity(count);
        let mut change = IdsChange::default();
        while out.len() < count {
            if k >= values.len() {
                values = sequences.get(seed, 2 * k + 16);
            }
            let value = values.get(k).copied().unwrap_or(0);
            k += 1;
            if value == 0 || self.contains(value) || change.values.contains(&value) {
                continue;
            }
            change.values.push(value);
            out.push(format!("{value:08X}"));
        }
        change.cursors.push((seed.to_owned(), k));
        (out, change)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_continue_after_the_last_value_held() {
        let sequences = Sequences::default();
        let second = sequence_value("word/document.xml", 1);
        let ids = Ids::opened([second].into_iter().collect());
        let (made, change) = ids.draw("word/document.xml", 1, &sequences);
        assert_eq!(made, vec![format!("{:08X}", sequence_value("word/document.xml", 2))]);
        let mut next = ids.clone();
        next.apply(&change);
        let (again, _) = next.draw("word/document.xml", 1, &sequences);
        assert_eq!(again, vec![format!("{:08X}", sequence_value("word/document.xml", 3))]);
        assert!(!ids.contains(sequence_value("word/document.xml", 2)));
    }
}
