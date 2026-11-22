//! Owns: [`SessionPages`], what a session lays its Word documents out with -- the fonts and
//! engine options ([`PageSetup`]) and the memory budget its engines count in --, the
//! session's default ([`crate::SessionConfig::pages`]) or one the host sets for a document
//! ([`Session::set_pages`]: a document drawn with other font folders or in another locale);
//! and each document's page engine ([`DocPages`]), made when a call first needs a layout its
//! host does not give ([`Session::pages`]), kept with the document and let go when it
//! closes.
//!
//! Must not: lay out (the engine does, when a tool asks), or make an engine for a document
//! that is not a Word document.

use crate::{DocId, MemoryBudget, Session, SessionError};
use ooxml_tools_word::{EngineBudgets, EnginePages, PageSetup, WordLayout};
use std::sync::{Arc, Mutex, PoisonError};

/// What a session lays its Word documents out with.
#[derive(Debug, Clone)]
pub struct SessionPages {
    /// The fonts and how the engines lay out.
    pub setup: PageSetup,
    /// The budget every engine counts in: its memos as held, its pages' paint within the
    /// limit.
    pub memory: Arc<MemoryBudget>,
}

impl SessionPages {
    fn engine(&self) -> Arc<EnginePages> {
        Arc::new(self.setup.engine(EngineBudgets {
            held: self.memory.holding(),
            paint: self.memory.account(),
        }))
    }
}

/// One Word document's engine, and what its host set it to be laid out with.
#[derive(Default)]
pub(crate) struct DocPages(Mutex<Held>);

#[derive(Default)]
struct Held {
    /// The host's setting for this document (`None`: the session's).
    own: Option<SessionPages>,
    engine: Option<Arc<EnginePages>>,
}

impl Session {
    /// The page engine the session lays the document `id` out with: made on first use from
    /// what [`Session::set_pages`] set for it, else the session's [`SessionPages`]; `None`
    /// where neither is given, or for a document that is not a Word document.
    ///
    /// # Errors
    /// `NotOpen` where no document `id` is open.
    pub fn pages(&self, id: &DocId) -> Result<Option<Arc<EnginePages>>, SessionError> {
        let document = self.document(id)?;
        let Some(word) = document.word() else {
            return Ok(None);
        };
        let mut held = word.pages.0.lock().unwrap_or_else(PoisonError::into_inner);
        if held.engine.is_none() {
            held.engine = held
                .own
                .as_ref()
                .or(self.config.pages.as_ref())
                .map(SessionPages::engine);
        }
        Ok(held.engine.clone())
    }

    /// Lay the document `id` out with `pages` from now on (`None`: with the session's): its
    /// engine is made again when next needed. A Word document only; another kind keeps none.
    ///
    /// # Errors
    /// `NotOpen` where no document `id` is open.
    pub fn set_pages(&self, id: &DocId, pages: Option<SessionPages>) -> Result<(), SessionError> {
        let document = self.document(id)?;
        if let Some(word) = document.word() {
            let mut held = word.pages.0.lock().unwrap_or_else(PoisonError::into_inner);
            *held = Held {
                own: pages,
                engine: None,
            };
        }
        Ok(())
    }

    /// The session's own layout of the document `id`, for a call whose host gives none.
    pub(crate) fn own_pages(&self, id: &DocId) -> Option<Arc<dyn WordLayout>> {
        let engine = self.pages(id).ok().flatten()?;
        Some(engine as Arc<dyn WordLayout>)
    }
}
