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

use crate::{Account, DocId, MemoryBudget, Session, SessionError};
use ooxml_derive::CacheBudget;
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
    /// A new engine of a document, and its accounts in the session's budget.
    fn engine(&self) -> (Arc<EnginePages>, [Arc<Account>; 2]) {
        let (held, paint) = (self.memory.holding(), self.memory.account());
        let engine = self.setup.engine(EngineBudgets {
            held: Arc::clone(&held) as _,
            paint: Arc::clone(&paint) as _,
        });
        (Arc::new(engine), [held, paint])
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
    /// The engine's accounts: what it holds of the version in use, and its pages' paint.
    accounts: Vec<Arc<Account>>,
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
            let made = held
                .own
                .as_ref()
                .or(self.config.pages.as_ref())
                .map(SessionPages::engine);
            if let Some((engine, accounts)) = made {
                held.engine = Some(engine);
                held.accounts = accounts.into();
            }
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
                ..Held::default()
            };
        }
        Ok(())
    }

    /// What the document `id` keeps in the session's memory budget now, bytes: its page
    /// engine's accounts (what it holds of the version in use, and its pages' paint); 0
    /// where it has no engine.
    ///
    /// # Errors
    /// `NotOpen` where no document `id` is open.
    pub fn document_memory(&self, id: &DocId) -> Result<usize, SessionError> {
        let document = self.document(id)?;
        let Some(word) = document.word() else {
            return Ok(0);
        };
        let held = word.pages.0.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(held.accounts.iter().map(|a| a.used()).sum())
    }

    /// The session's own layout of the document `id`, for a call whose host gives none.
    pub(crate) fn own_pages(&self, id: &DocId) -> Option<Arc<dyn WordLayout>> {
        let engine = self.pages(id).ok().flatten()?;
        Some(engine as Arc<dyn WordLayout>)
    }
}
