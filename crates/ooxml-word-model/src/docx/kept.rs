//! Owns: [`Kept`], what one document's versions read of each story part, as the latest
//! version that read it left it -- the paragraphs' texts ([`PartTexts`]) and the lookups'
//! facts ([`PartFacts`]) -- shared by all its versions, so a version reads from them
//! whatever the versions between it and that one read (the paragraphs and blocks it shares
//! with that version, by pointer, are not read again). A part's first reading is shared
//! too: versions that find nothing kept wait for the one reading it and then read from it,
//! instead of each reading the whole part at once.
//!
//! Also owns what the document's versions name their stories by ([`StoryTable`], one per
//! document: a block's key is the same in every version, whoever reads it) and the block
//! tier of its projections ([`crate::projection::Projections`]).
//!
//! Must not: keep anything but the latest reading of each part; hold a lock other than its
//! own part's first-reading gate while reading.

use super::block_facts::BlockFacts;
use super::facts::PartFacts;
use super::keeps::RowKeep;
use super::kept_rows::RowsMemo;
use super::texts::PartTexts;
use crate::ids::{StoryIx, StoryTable};
use crate::projection::Projections;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The latest reading of each part, by part name, and each part's first-reading gate.
#[derive(Debug)]
struct Latest<T> {
    read: Mutex<HashMap<String, Arc<T>>>,
    first: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl<T> Default for Latest<T> {
    fn default() -> Self {
        Latest {
            read: Mutex::new(HashMap::new()),
            first: Mutex::new(HashMap::new()),
        }
    }
}

impl<T> Latest<T> {
    fn get(&self, part: &str) -> Option<Arc<T>> {
        self.read
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(part)
            .cloned()
    }

    fn keep(&self, part: &str, made: T) -> Arc<T> {
        let made = Arc::new(made);
        self.read
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(part.to_owned(), Arc::clone(&made));
        made
    }

    fn gate(&self, part: &str) -> Arc<Mutex<()>> {
        let mut gates = self.first.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(gates.entry(part.to_owned()).or_default())
    }

    /// `part` as `read` reads it from the latest reading, kept as the latest in turn. With
    /// none kept yet, one caller at a time reads under the part's gate, so callers that
    /// arrive meanwhile read from the first one's reading.
    fn read_with(&self, part: &str, read: impl FnOnce(Option<&T>) -> T) -> Arc<T> {
        if let Some(latest) = self.get(part) {
            return self.keep(part, read(Some(&latest)));
        }
        let gate = self.gate(part);
        let _first = gate.lock().unwrap_or_else(PoisonError::into_inner);
        let latest = self.get(part);
        self.keep(part, read(latest.as_deref()))
    }
}

/// One document's latest readings per story part, shared by its versions, counted on the
/// document's memory budget (no limit of their own).
#[derive(Debug)]
pub struct Kept {
    texts: Latest<PartTexts>,
    facts: Latest<PartFacts>,
    /// Each table's rows' keeps, by the table element, the outermost table holding it and
    /// the parts they were read with, across the versions that share them.
    rows: RowsMemo,
    /// Each block's facts its readers name, by the block element, the outermost table
    /// holding it and the parts they were made with, across the versions that share them.
    blocks: BlockFacts,
    /// The document's stories, interned once: what its versions' blocks are keyed by.
    stories: Mutex<StoryTable>,
    /// Its blocks' projections, by the block element, the view and what the reading read
    /// beside it, across the versions that share them.
    projections: Projections,
}

impl Kept {
    /// Nothing kept yet; the readings counted on `budget` (the document's account).
    #[must_use]
    pub fn new(budget: Arc<dyn ooxml_derive::CacheBudget>) -> Kept {
        Kept {
            texts: Latest::default(),
            facts: Latest::default(),
            rows: RowsMemo::new(Arc::clone(&budget)),
            blocks: BlockFacts::new(Arc::clone(&budget)),
            stories: Mutex::new(StoryTable::new()),
            projections: Projections::new(budget),
        }
    }

    /// What the blocks' projections hold (the block tier: a table's cells are read from
    /// it, [`super::cells::TableFacts`]): entries and bytes.
    #[must_use]
    pub fn block_readings(&self) -> (usize, usize) {
        let stats = self.projections.stats();
        (stats.entries, stats.bytes)
    }

    /// What the tables' rows' keeps hold: entries and bytes.
    #[must_use]
    pub fn row_keeps(&self) -> (usize, usize) {
        let stats = self.rows.stats();
        (stats.entries, stats.bytes)
    }

    /// `part`'s texts as `read` reads them from the latest texts a version of it read
    /// (`None`: no version has yet), kept as the latest.
    pub fn texts(&self, part: &str, read: impl FnOnce(Option<&PartTexts>) -> PartTexts) -> Arc<PartTexts> {
        self.texts.read_with(part, read)
    }

    /// `part`'s facts as `read` reads them from the latest facts a version of it read
    /// (`None`: no version has yet), kept as the latest.
    pub fn facts(&self, part: &str, read: impl FnOnce(Option<&PartFacts>) -> PartFacts) -> Arc<PartFacts> {
        self.facts.read_with(part, read)
    }

    /// The document's stories: what its versions' blocks are keyed by.
    pub(crate) fn stories(&self) -> MutexGuard<'_, StoryTable> {
        self.stories.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The index of the story `name`: `None` past the table's limit.
    pub(crate) fn story(&self, name: &str) -> Option<StoryIx> {
        self.stories().intern(name)
    }

    /// The projections kept across the versions.
    pub(crate) fn projections(&self) -> &Projections {
        &self.projections
    }

    /// The blocks' facts kept across the versions.
    pub(crate) fn blocks(&self) -> &BlockFacts {
        &self.blocks
    }

    /// The rows' keeps of `table` (in `outer`, the outermost table holding it) with
    /// `formatting`, as kept across the versions, else made by `read` and kept.
    pub(crate) fn rows(
        &self,
        formatting: &super::formatting::Formatting,
        table: &Arc<ooxml_xml::Element>,
        outer: &Arc<ooxml_xml::Element>,
        read: impl FnOnce() -> Vec<RowKeep>,
    ) -> Arc<[RowKeep]> {
        self.rows.rows(formatting, table, outer, read)
    }
}

#[cfg(test)]
mod tests {
    use super::Latest;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn concurrent_first_readers_read_from_one_first_reading() {
        let latest = Arc::new(Latest::<usize>::default());
        let whole = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(Barrier::new(8));
        let readers: Vec<_> = (0..8)
            .map(|_| {
                let (latest, whole, start) = (Arc::clone(&latest), Arc::clone(&whole), Arc::clone(&start));
                thread::spawn(move || {
                    start.wait();
                    *latest.read_with("word/document.xml", |earlier| match earlier {
                        Some(read) => read + 1,
                        None => {
                            whole.fetch_add(1, Ordering::SeqCst);
                            thread::sleep(Duration::from_millis(50));
                            1
                        }
                    })
                })
            })
            .collect();
        let mut got: Vec<usize> = readers.into_iter().map(|r| r.join().unwrap()).collect();
        got.sort_unstable();
        assert_eq!(whole.load(Ordering::SeqCst), 1, "one reading of the whole part");
        assert_eq!(got, (1..=8).collect::<Vec<_>>());
    }

    #[test]
    fn a_kept_reading_is_read_from_without_the_gate() {
        let latest = Latest::<usize>::default();
        assert_eq!(*latest.read_with("p", |e| e.map_or(10, |r| r + 1)), 10);
        let gate = latest.gate("p");
        let _held = gate.lock().unwrap();
        assert_eq!(*latest.read_with("p", |e| e.map_or(10, |r| r + 1)), 11);
    }
}
