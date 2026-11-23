//! Owns: [`Derived`], what is read from one version once and kept with its [`super::Docx`]
//! for every call on that version: the lookups a Markdown reading starts from, each
//! Markdown read made (by range, view and stories: the pages after the first, and a read
//! again, take it as it is), and what a story's window reads keep (its blocks grouped in a
//! view, and the list counters at places along it: `markdown::read::window`), and its
//! sections (`super::sections::sections`, which a field's page and a table of contents
//! read for every field), and each part's relationships (a hyperlink, a diagram and a
//! section's header read them by id) -- with the
//! version before's, which a window read takes what is still true of from.
//!
//! Must not: be shared between versions (a new version's reading starts empty but for the
//! version before's window readings, kept to be checked, never used unchecked), or hold
//! anything a version's content does not determine.

use crate::markdown::read::{Kept, Shared};
use crate::markdown::{RangeError, Stories, ViewName};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// A Markdown read's key: its range, view and stories.
pub type MarkdownKey = (Option<String>, ViewName, Stories);

/// A story's window readings' key: its part and the view.
pub type WindowKey = (String, ViewName);

/// A part's relationships as read, or why they could not be.
type RelationshipsRead = Result<Arc<crate::parts::PartRelationships>, ooxml_opc::OpcError>;

/// Markdown reads kept per version, at most.
const READS_KEPT: usize = 16;

/// One version's derived readings, each made once when first asked.
#[derive(Debug, Default)]
pub struct Derived {
    reading: OnceLock<Arc<Shared>>,
    sections: OnceLock<Vec<super::sections::Section>>,
    /// Each part's relationships, read once ([`crate::parts::PartRelationships`]).
    relationships: Mutex<HashMap<String, RelationshipsRead>>,
    markdown: Mutex<HashMap<MarkdownKey, Arc<String>>>,
    windows: Mutex<HashMap<WindowKey, Arc<Kept>>>,
    /// The version before's window readings.
    earlier: HashMap<WindowKey, Arc<Kept>>,
}

impl Clone for Derived {
    /// A copy starts empty: what it derives is the copy's own.
    fn clone(&self) -> Self {
        Derived::default()
    }
}

impl Derived {
    /// A new version's: empty, with `before`'s window readings to check against it.
    #[must_use]
    pub fn after(before: Option<&Derived>) -> Derived {
        let earlier = before.map_or_else(HashMap::new, |b| {
            let mut kept = b.earlier.clone();
            kept.extend(
                b.windows
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .iter()
                    .map(|(k, v)| (k.clone(), Arc::clone(v))),
            );
            kept
        });
        Derived {
            earlier,
            ..Derived::default()
        }
    }

    /// The relationships of the part `source` of `docx`'s package, read once.
    ///
    /// # Errors
    /// Its relationships part is not XML.
    pub(crate) fn relationships(&self, docx: &super::Docx, source: &str) -> RelationshipsRead {
        if let Some(found) = self
            .relationships
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(source)
        {
            return found.clone();
        }
        let read = crate::parts::PartRelationships::read(&docx.package, source).map(Arc::new);
        self.relationships
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(source.to_owned(), read.clone());
        read
    }

    /// The version's sections (`docx`'s), read once.
    pub(crate) fn sections(&self, docx: &super::Docx) -> &[super::sections::Section] {
        self.sections.get_or_init(|| super::sections::sections(docx))
    }

    /// The window readings of `key` this version keeps, or else the version before's (to
    /// be checked), and which of the two they are (`true`: this version's).
    pub(crate) fn window(&self, key: &WindowKey) -> Option<(Arc<Kept>, bool)> {
        let own = self
            .windows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned();
        own.map(|k| (k, true))
            .or_else(|| self.earlier.get(key).map(|k| (Arc::clone(k), false)))
    }

    /// Keep `kept` as this version's window readings of `key`.
    pub(crate) fn keep_window(&self, key: WindowKey, kept: Arc<Kept>) {
        self.windows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, kept);
    }

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
