//! Owns: [`Derived`], what is read from one version once and kept with its [`super::Docx`]
//! for every call on that version: the lookups a Markdown reading starts from, and each
//! Markdown read made (by range, view and stories: the pages after the first, and a read
//! again, take it as it is).
//!
//! Must not: be shared between versions (a new version's reading starts empty), or hold
//! anything a version's content does not determine.

use crate::markdown::read::Shared;
use crate::markdown::{RangeError, Stories, ViewName};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// A Markdown read's key: its range, view and stories.
pub type MarkdownKey = (Option<String>, ViewName, Stories);

/// Markdown reads kept per version, at most.
const READS_KEPT: usize = 16;

/// One version's derived readings, each made once when first asked.
#[derive(Debug, Default)]
pub struct Derived {
    reading: OnceLock<Arc<Shared>>,
    markdown: Mutex<HashMap<MarkdownKey, Arc<String>>>,
}

impl Clone for Derived {
    /// A copy starts empty: what it derives is the copy's own.
    fn clone(&self) -> Self {
        Derived::default()
    }
}

impl Derived {
    /// The lookups a reading starts from, made by `read` the first time.
    pub fn reading(&self, read: impl FnOnce() -> Shared) -> Arc<Shared> {
        Arc::clone(self.reading.get_or_init(|| Arc::new(read())))
    }

    /// The Markdown read `key`, made by `read` the first time it succeeds (a few kept: past
    /// that many, those kept are let go and the count starts again).
    ///
    /// # Errors
    /// `read`'s.
    pub fn markdown(
        &self,
        key: MarkdownKey,
        read: impl FnOnce() -> Result<String, RangeError>,
    ) -> Result<Arc<String>, RangeError> {
        if let Some(found) = self.markdown.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
            return Ok(Arc::clone(found));
        }
        let made = Arc::new(read()?);
        let mut kept = self.markdown.lock().unwrap_or_else(PoisonError::into_inner);
        if kept.len() >= READS_KEPT {
            kept.clear();
        }
        kept.insert(key, Arc::clone(&made));
        Ok(made)
    }
}
