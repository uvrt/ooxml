//! Owns: [`Derived`], what is read from one version once and kept with its [`super::Docx`]
//! for every call on that version: the lookups a Markdown reading starts from.
//!
//! Must not: be shared between versions (a new version's reading starts empty), or hold
//! anything a version's content does not determine.

use crate::markdown::read::Shared;
use std::sync::{Arc, OnceLock};

/// One version's derived readings, each made once when first asked.
#[derive(Debug, Default)]
pub struct Derived {
    reading: OnceLock<Arc<Shared>>,
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
}
