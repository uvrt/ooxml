//! Owns: [`Kept`], what one document's versions read of each story part, as the latest
//! version that read it left it -- the paragraphs' texts ([`PartTexts`]) and the lookups'
//! facts ([`PartFacts`]) -- shared by all its versions, so a version reads from them
//! whatever the versions between it and that one read (the paragraphs and blocks it shares
//! with that version, by pointer, are not read again). A part's first reading is shared
//! too: versions that find nothing kept wait for the one reading it and then read from it,
//! instead of each reading the whole part at once.
//!
//! Must not: keep anything but the latest reading of each part; hold a lock other than its
//! own part's first-reading gate while reading.

use super::cells::TableMemo;
use super::facts::PartFacts;
use super::texts::PartTexts;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

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

/// One document's latest readings per story part, shared by its versions.
#[derive(Debug, Default)]
pub struct Kept {
    texts: Latest<PartTexts>,
    facts: Latest<PartFacts>,
    /// Each table's reading (its paragraphs' cells), by the table element and the parts
    /// it was read with, across the versions that share them.
    tables: TableMemo,
}

impl Kept {
    /// Nothing kept yet; the tables' readings counted on `budget` (the document's account).
    #[must_use]
    pub fn new(budget: Arc<dyn ooxml_derive::CacheBudget>) -> Kept {
        Kept {
            texts: Latest::default(),
            facts: Latest::default(),
            tables: TableMemo::new(budget),
        }
    }

    /// What the tables' readings hold: entries and bytes.
    #[must_use]
    pub fn table_readings(&self) -> (usize, usize) {
        let stats = self.tables.stats();
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

    /// The tables' readings kept across the versions.
    pub(crate) fn tables(&self) -> &TableMemo {
        &self.tables
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
