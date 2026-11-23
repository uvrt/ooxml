//! Owns: [`Detections`], each paragraph text's detected language kept across a document's
//! checks: [`detect`] reads only the text, so a text checked before is not detected again
//! -- a check after a local edit detects the edited paragraphs alone. What a check of the
//! whole document did not meet goes with it (a text no version has any more is dropped).
//!
//! Must not: keep anything but texts and what [`detect`] found in them.

use super::detect::{detect, Detected};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// Each text's language as [`detect`] found it, kept across a document's checks.
#[derive(Debug, Default)]
pub struct Detections {
    kept: Mutex<Arc<HashMap<String, Detected>>>,
    /// Each paragraph's judgement, by the elements it read.
    pub(super) paragraphs: super::judged::Judgements,
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
            Arc::clone(&m.kept.lock().unwrap_or_else(PoisonError::into_inner))
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
        *memo.kept.lock().unwrap_or_else(PoisonError::into_inner) = Arc::new(kept);
    }
}
