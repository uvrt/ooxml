//! Owns: a version's ids as edits draw them -- docx-agent's pool and ledger
//! (`edit/ids.py`, `generate`) as a persistent value: every `ST_LongHexNumber` value any
//! state had or a draw gave, where each seed's sequence got to, and the renames
//! ([`Ids`]); new values drawn deterministically from a seed's SHA-256 sequence, so the
//! same edit writes the same bytes ([`Ids::draw`]); the sequences themselves, computed once
//! per document ([`Sequences`]).
//!
//! An edit draws stand-ins (`Change::Draw`, [`crate::Drawn`]): the values are drawn again
//! where its change is applied, so two transactions drawing from one seed do not conflict
//! and each gets what a serial run in commit order gives (design section 5's ledger, merged
//! at commit). A stand-in written where no draw reaches is pinned: the edit reads its seed's
//! key (`Key::Seq`), and conflicts with any draw from it since its snapshot.
//!
//! Must not: stamp a part (the sink does), read a package, or hold global state.

use ooxml_word_model::ids::{sequence_value, REACH};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

/// The key drawing numbers are drawn under (`Key::Seq`): two transactions drawing them
/// are ordered by their commits, as two drawing from one seed are.
pub const DRAWING_NUMBERS: &str = "\0drawing numbers";

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
    /// The highest drawing number given now (`wp:docPr/@id`), where a draw gave one or a
    /// part brought one above the version's ([`Ids::highest_drawing`]); 0 where neither.
    pub highest_drawing: u64,
    /// Drawing numbers given now (drawn, kept by a copy, or brought by parts) that the
    /// document had not had ([`Ids::had_drawing`]).
    pub drawings: Vec<u64>,
    /// Blocks replaced, one by one (docx-agent's `_note_replaced`): the old block id's
    /// value and the id of the block that replaced it.
    pub replaced: Vec<(u32, String)>,
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
            && self.highest_drawing == 0
            && self.replaced.is_empty()
            && self.drawings.is_empty()
    }
}

/// The ids a document had before it was saved and opened again, as the host kept them
/// beside its bytes (docx-agent's id state, round 38): every value its pool held -- in use
/// and retired --, where each seed's generation got to, and the highest drawing number
/// given. A document opened with them never gives a retired id again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EarlierIds {
    /// Every value the pool held.
    pub values: Vec<u32>,
    /// Seeds and where their generation got to.
    pub cursors: Vec<(String, usize)>,
    /// The highest drawing number given.
    pub highest_drawing: u64,
    /// The drawing numbers retired (docx-agent's `drawings_gone`): a copy keeps none of them.
    pub drawings: Vec<u64>,
}

/// A version's ids: the pool, the sequences' cursors and the renames. Cloning shares.
#[derive(Debug, Clone, Default)]
pub struct Ids {
    base: Arc<HashSet<u32>>,
    added: Arc<BTreeSet<u32>>,
    cursors: Arc<BTreeMap<String, usize>>,
    aliases: Arc<BTreeMap<String, String>>,
    /// The highest drawing number any version had or a draw gave (docx-agent's ledger's
    /// `highest_drawing`): every `wp:docPr/@id` written is at most this.
    highest_drawing: u64,
    /// Every drawing number any version had or a draw gave (docx-agent's ledger's drawing
    /// numbers in the document and `drawings_gone`): a copy keeps a number only outside it.
    drawings: Arc<BTreeSet<u64>>,
    /// Each block id's value replaced by one block, and that block's id (docx-agent's
    /// ledger's `replaced`: what a refusal naming the old id says replaced it).
    replaced: Arc<BTreeMap<u32, String>>,
    /// The first annotation id of the edit being applied (one above the largest in use
    /// when it began numbering): its revision records' ids count from here.
    annotations: Option<i64>,
}

impl Ids {
    /// The ids of a document as opened: `values`, every one its parts hold, and
    /// `drawings`, the drawing numbers they write.
    #[must_use]
    pub fn opened(values: HashSet<u32>, drawings: HashSet<u64>) -> Ids {
        Ids {
            base: Arc::new(values),
            highest_drawing: drawings.iter().copied().max().unwrap_or(0),
            drawings: Arc::new(drawings.into_iter().collect()),
            ..Ids::default()
        }
    }

    /// These ids with what the document had before it was saved and opened again
    /// (`earlier`): its values in the pool, its cursors, its highest drawing number.
    #[must_use]
    pub fn with_earlier(mut self, earlier: &EarlierIds) -> Ids {
        if !earlier.values.is_empty() {
            Arc::make_mut(&mut self.base).extend(earlier.values.iter().copied());
        }
        if !earlier.cursors.is_empty() {
            Arc::make_mut(&mut self.cursors).extend(earlier.cursors.iter().cloned());
        }
        self.highest_drawing = self.highest_drawing.max(earlier.highest_drawing);
        if !earlier.drawings.is_empty() {
            Arc::make_mut(&mut self.drawings).extend(earlier.drawings.iter().copied());
        }
        self
    }

    /// Whether the document has or had the drawing number `number`, or a draw gave it.
    #[must_use]
    pub fn had_drawing(&self, number: u64) -> bool {
        self.drawings.contains(&number)
    }

    /// The highest drawing number any version had or a draw gave.
    #[must_use]
    pub fn highest_drawing(&self) -> u64 {
        self.highest_drawing
    }

    /// `count` new drawing numbers (docx-agent's `_next_drawing_id` and importer's
    /// `_drawing_ids` for a number in use): each one past every number the document has
    /// and had (a drawing's number is never another drawing's); and the change that
    /// records them.
    #[must_use]
    pub fn draw_drawings(&self, count: usize) -> (Vec<u64>, IdsChange) {
        let first = self.highest_drawing.saturating_add(1);
        let out: Vec<u64> = (0..count as u64).map(|k| first.saturating_add(k)).collect();
        let change = IdsChange {
            highest_drawing: out.last().copied().unwrap_or(0),
            drawings: out.clone(),
            ..IdsChange::default()
        };
        (out, change)
    }

    /// Whether `value` is in the pool.
    #[must_use]
    pub fn contains(&self, value: u32) -> bool {
        self.base.contains(&value) || self.added.contains(&value)
    }

    /// The values given since the document was opened (by its edits, or brought by parts
    /// the bridge committed): what a host that keeps a view of the document learns from
    /// the session, never by reading its parts again.
    #[must_use]
    pub fn added(&self) -> &BTreeSet<u32> {
        &self.added
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

    /// Each block id's value replaced by one block, with that block's id.
    #[must_use]
    pub fn replaced(&self) -> &BTreeMap<u32, String> {
        &self.replaced
    }

    /// Add what `change` adds.
    pub fn apply(&mut self, change: &IdsChange) {
        if !change.replaced.is_empty() {
            Arc::make_mut(&mut self.replaced).extend(change.replaced.iter().cloned());
        }
        self.highest_drawing = self.highest_drawing.max(change.highest_drawing);
        if change.drawings.iter().any(|n| !self.drawings.contains(n)) {
            Arc::make_mut(&mut self.drawings).extend(change.drawings.iter().copied());
        }
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

    /// Take in what `head`'s ledger holds: every value it gives as used, each sequence's
    /// cursor where it got furthest, and its renames (docx-agent's ledger and aliases belong
    /// to the document, not to a version: an undo or redo keeps them, so an id given is
    /// never given again and an old id still names the block it was renamed to).
    pub fn carry(&mut self, head: &Ids) {
        self.highest_drawing = self.highest_drawing.max(head.highest_drawing);
        if !Arc::ptr_eq(&self.drawings, &head.drawings) {
            let mut drawings = (*self.drawings).clone();
            drawings.extend(head.drawings.iter().copied());
            self.drawings = Arc::new(drawings);
        }
        if !Arc::ptr_eq(&self.added, &head.added) {
            let mut added = (*self.added).clone();
            added.extend(head.added.iter().copied());
            if !Arc::ptr_eq(&self.base, &head.base) {
                added.extend(head.base.iter().copied().filter(|v| !self.base.contains(v)));
            }
            self.added = Arc::new(added);
        }
        if !Arc::ptr_eq(&self.cursors, &head.cursors) {
            let cursors = Arc::make_mut(&mut self.cursors);
            for (seed, k) in head.cursors.iter() {
                let at = cursors.entry(seed.clone()).or_insert(*k);
                *at = (*at).max(*k);
            }
        }
        if !Arc::ptr_eq(&self.aliases, &head.aliases) {
            let aliases = Arc::make_mut(&mut self.aliases);
            for (old, new) in head.aliases.iter() {
                aliases.insert(old.clone(), new.clone());
            }
        }
        if !Arc::ptr_eq(&self.replaced, &head.replaced) {
            let replaced = Arc::make_mut(&mut self.replaced);
            for (old, new) in head.replaced.iter() {
                replaced.insert(*old, new.clone());
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
        let ids = Ids::opened([second].into_iter().collect(), HashSet::new());
        let (made, change) = ids.draw("word/document.xml", 1, &sequences);
        assert_eq!(made, vec![format!("{:08X}", sequence_value("word/document.xml", 2))]);
        let mut next = ids.clone();
        next.apply(&change);
        let (again, _) = next.draw("word/document.xml", 1, &sequences);
        assert_eq!(again, vec![format!("{:08X}", sequence_value("word/document.xml", 3))]);
        assert!(!ids.contains(sequence_value("word/document.xml", 2)));
    }

    #[test]
    fn an_id_retired_before_the_document_was_saved_is_never_drawn_again() {
        let sequences = Sequences::default();
        let seed = "word/document.xml";
        // the parts hold the first value; the second was given and deleted before the save
        let opened = Ids::opened(
            [sequence_value(seed, 1)].into_iter().collect(),
            [3].into_iter().collect(),
        );
        let earlier = EarlierIds {
            values: vec![sequence_value(seed, 2)],
            cursors: Vec::new(),
            highest_drawing: 7,
            drawings: vec![5],
        };
        let ids = opened.clone().with_earlier(&earlier);
        let (again, _) = opened.draw(seed, 1, &sequences);
        let (made, _) = ids.draw(seed, 1, &sequences);
        assert_eq!(again, vec![format!("{:08X}", sequence_value(seed, 2))]);
        assert_eq!(made, vec![format!("{:08X}", sequence_value(seed, 3))]);
        assert_eq!(ids.highest_drawing(), 7);
        assert!(ids.had_drawing(5) && ids.had_drawing(3) && !ids.had_drawing(4));
        assert!(ids.added().is_empty());
    }
}
