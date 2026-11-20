//! Owns: [`Kept`], what one document's versions read of each story part, as the latest
//! version that read it left it -- the paragraphs' texts ([`PartTexts`]) and the lookups'
//! facts ([`PartFacts`]) -- shared by all its versions, so a version reads from them
//! whatever the versions between it and that one read (the paragraphs and blocks it shares
//! with that version, by pointer, are not read again).
//!
//! Must not: keep anything but the latest reading of each part.

use super::facts::PartFacts;
use super::texts::PartTexts;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// The latest reading of each part, by part name.
#[derive(Debug)]
struct Latest<T>(Mutex<HashMap<String, Arc<T>>>);

impl<T> Default for Latest<T> {
    fn default() -> Self {
        Latest(Mutex::new(HashMap::new()))
    }
}

impl<T> Latest<T> {
    fn get(&self, part: &str) -> Option<Arc<T>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).get(part).cloned()
    }

    fn put(&self, part: &str, read: &Arc<T>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(part.to_owned(), Arc::clone(read));
    }
}

/// One document's latest readings per story part, shared by its versions.
#[derive(Debug, Default)]
pub struct Kept {
    texts: Latest<PartTexts>,
    facts: Latest<PartFacts>,
}

impl Kept {
    /// The texts of `part` that a version of it read last.
    #[must_use]
    pub fn texts(&self, part: &str) -> Option<Arc<PartTexts>> {
        self.texts.get(part)
    }

    /// Keep `texts` as `part`'s latest.
    pub fn keep_texts(&self, part: &str, texts: &Arc<PartTexts>) {
        self.texts.put(part, texts);
    }

    /// The facts of `part` that a version of it read last.
    #[must_use]
    pub fn facts(&self, part: &str) -> Option<Arc<PartFacts>> {
        self.facts.get(part)
    }

    /// Keep `facts` as `part`'s latest.
    pub fn keep_facts(&self, part: &str, facts: &Arc<PartFacts>) {
        self.facts.put(part, facts);
    }
}
