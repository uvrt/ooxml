//! Owns: [`Detections`], each paragraph text's detected language kept across a document's
//! checks: [`detect`] reads only the text, so a text checked before is not detected again
//! -- a check after a local edit detects the edited paragraphs alone. What a check of the
//! whole document did not meet goes with it (a text no version has any more is dropped).
//! The texts are held within a budget (the document's account in a session; a fixed bound
//! otherwise): what one check keeps is reserved whole, and where the budget refuses it
//! nothing is kept (the next check detects every text again).
//!
//! Must not: keep anything but texts and what [`detect`] found in them, or hold texts the
//! budget did not give room for.

use super::detect::{detect, Detected};
use super::judged::JUDGEMENT_BYTES;
use ooxml_derive::{CacheBudget, Limit, MemoStats};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// The texts kept and the bytes reserved for them.
#[derive(Debug, Default)]
struct Texts {
    by_text: Arc<HashMap<String, Detected>>,
    bytes: usize,
}

/// Each text's language as [`detect`] found it, kept across a document's checks.
pub struct Detections {
    kept: Mutex<Texts>,
    budget: Arc<dyn CacheBudget>,
    /// Each paragraph's judgement, by the elements it read.
    pub(super) paragraphs: super::judged::Judgements,
}

impl std::fmt::Debug for Detections {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Detections")
            .field("texts", &self.texts())
            .field("paragraphs", &self.paragraphs)
            .finish()
    }
}

impl Default for Detections {
    fn default() -> Detections {
        Detections::new(Arc::new(Limit::new(2 * JUDGEMENT_BYTES)))
    }
}

impl Drop for Detections {
    fn drop(&mut self) {
        let texts = self.kept.get_mut().unwrap_or_else(PoisonError::into_inner);
        self.budget.release(texts.bytes);
    }
}

/// What one kept text holds, bytes, roughly: the text, its entry and the map's share.
fn weight(text: &str) -> usize {
    text.len() + std::mem::size_of::<(String, Detected)>() + 8
}

impl Detections {
    /// Detections whose texts and paragraphs' judgements are counted on `budget` (a
    /// document's account).
    #[must_use]
    pub fn new(budget: Arc<dyn CacheBudget>) -> Detections {
        Detections {
            kept: Mutex::default(),
            paragraphs: super::judged::Judgements::new(Arc::clone(&budget)),
            budget,
        }
    }

    /// What the paragraphs' judgements hold.
    #[must_use]
    pub fn judgements(&self) -> MemoStats {
        self.paragraphs.stats()
    }

    /// What the texts' detections hold: the texts and the bytes reserved for them.
    #[must_use]
    pub fn texts(&self) -> MemoStats {
        let kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        MemoStats {
            entries: kept.by_text.len(),
            bytes: kept.bytes,
            ..MemoStats::default()
        }
    }

    /// Keep `by_text` in place of what was kept, where the budget gives room for it; else
    /// keep nothing.
    fn keep(&self, by_text: HashMap<String, Detected>) {
        let bytes: usize = by_text.keys().map(|t| weight(t)).sum();
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        self.budget.release(kept.bytes);
        *kept = if self.budget.reserve(bytes) {
            Texts {
                by_text: Arc::new(by_text),
                bytes,
            }
        } else {
            Texts::default()
        };
    }
}

/// One check's detections: the kept ones read, the ones met kept for the next check.
pub(super) struct Detecting<'d> {
    memo: Option<&'d Detections>,
    before: Arc<HashMap<String, Detected>>,
    met: HashMap<String, Detected>,
    whole: bool,
}

impl<'d> Detecting<'d> {
    /// A check's detections, with `memo`'s kept ones where given; `whole`: the check meets
    /// every paragraph (what it does not meet is dropped).
    pub(super) fn new(memo: Option<&'d Detections>, whole: bool) -> Detecting<'d> {
        let before = memo.map_or_else(Arc::default, |m| {
            Arc::clone(&m.kept.lock().unwrap_or_else(PoisonError::into_inner).by_text)
        });
        Detecting {
            memo,
            before,
            met: HashMap::new(),
            whole,
        }
    }

    /// `text`'s language: as detected before, else detected now.
    pub(super) fn detect(&mut self, text: &str) -> Detected {
        if let Some(found) = self.met.get(text).or_else(|| self.before.get(text)) {
            let found = *found;
            if self.memo.is_some() {
                self.met.insert(text.to_owned(), found);
            }
            return found;
        }
        let found = detect(text);
        if self.memo.is_some() {
            self.met.insert(text.to_owned(), found);
        }
        found
    }

    /// Keep what this check met for the next one: only that where it met every paragraph,
    /// else added to what was kept.
    pub(super) fn finish(self) {
        let Some(memo) = self.memo else { return };
        let kept = if self.whole {
            self.met
        } else {
            let mut all = (*self.before).clone();
            all.extend(self.met);
            all
        };
        memo.keep(kept);
    }
}
