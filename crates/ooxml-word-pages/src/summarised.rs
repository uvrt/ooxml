//! Owns: a version's summary as a comparison holds it ([`Summarised`]): a version laid
//! out by an engine, whose pages are digested only when a comparison needs them (and then
//! only the pages the two versions do not share), or a summary made whole. What a check
//! saw is held so, for the next check to compare with.
//!
//! Must not: lock an engine while another is locked, or paint but through the engine.

use crate::summary::{Reflow, Summary};
use crate::{LaidVersion, PageEngine};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// A version's summary as a comparison holds it.
#[derive(Clone)]
pub enum Summarised {
    /// A version laid out by `engine`: digested when compared.
    Laid {
        /// The version.
        version: Arc<LaidVersion>,
        /// The engine that laid it out.
        engine: Arc<Mutex<PageEngine>>,
    },
    /// A summary made whole.
    Made(Arc<Summary>),
}

impl std::fmt::Debug for Summarised {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Summarised::Laid { version, .. } => f.debug_tuple("Laid").field(version).finish(),
            Summarised::Made(summary) => f.debug_tuple("Made").field(&summary.count()).finish(),
        }
    }
}

fn lock(engine: &Mutex<PageEngine>) -> std::sync::MutexGuard<'_, PageEngine> {
    engine.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Summarised {
    /// `version`, laid out by `engine`, kept comparable there ([`PageEngine::watch`]):
    /// `engine` is locked for it, so it must not be locked by the caller.
    pub fn watched(version: Arc<LaidVersion>, engine: Arc<Mutex<PageEngine>>) -> Summarised {
        lock(&engine).watch(&version);
        Summarised::Laid { version, engine }
    }

    /// Its pages.
    pub fn count(&self) -> usize {
        match self {
            Summarised::Laid { version, .. } => version.page_count(),
            Summarised::Made(summary) => summary.count(),
        }
    }

    /// Its pages known.
    pub fn known(&self) -> usize {
        match self {
            Summarised::Laid { version, .. } => version.page_count(),
            Summarised::Made(summary) => summary.known,
        }
    }

    /// Whether it is `other`: the very version, or the very summary.
    pub fn is(&self, other: &Summarised) -> bool {
        match (self, other) {
            (Summarised::Laid { version: a, .. }, Summarised::Laid { version: b, .. }) => Arc::ptr_eq(a, b),
            (Summarised::Made(a), Summarised::Made(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Its summary whole, made where it was not (every page its engine has not digested
    /// painted); `None` where its engine no longer paints it.
    pub fn summary(&self) -> Option<Arc<Summary>> {
        match self {
            Summarised::Laid { version, engine } => match version.summary.get() {
                Some(made) => Some(Arc::clone(made)),
                None => lock(engine).summary(version),
            },
            Summarised::Made(summary) => Some(Arc::clone(summary)),
        }
    }

    /// What changed from `before` to this one ([`Summary::compare`]), ids followed through
    /// `aliases`: two versions of one engine compared page by page by it, else their
    /// summaries whole. `None` where either can no longer be digested.
    pub fn compare(&self, before: &Summarised, aliases: &HashMap<String, String>) -> Option<Reflow> {
        if let (
            Summarised::Laid { version: now, engine },
            Summarised::Laid {
                version: was,
                engine: theirs,
            },
        ) = (self, before)
        {
            if Arc::ptr_eq(engine, theirs) {
                return lock(engine).compare(now, was, aliases);
            }
        }
        let was = before.summary()?;
        Some(self.summary()?.compare(&was, aliases))
    }
}
