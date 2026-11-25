//! Owns: [`SessionPages`], what a session lays its Word documents out with -- the fonts and
//! engine options ([`PageSetup`]) and the memory budget its engines count in --, the
//! session's default ([`crate::WordFormat::pages`]) or one the host sets for a document
//! ([`WordPages::set_pages`]: a document drawn with other font folders or in another locale);
//! and each document's page engine ([`DocPages`]), made when a call first needs a layout its
//! host does not give ([`WordPages::pages`]), kept with the document and let go when it
//! closes.
//!
//! Must not: lay out (the engine does, when a tool asks), or make an engine for a document
//! that is not a Word document.

use crate::word::Word;
use ooxml_derive::CacheBudget;
use ooxml_session::{Account, DocId, MemoryBudget, Session, SessionError};
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

/// A session's page engines of its Word documents.
pub trait WordPages {
    /// The page engine the session lays the document `id` out with: made on first use from
    /// what [`WordPages::set_pages`] set for it, else the session's [`SessionPages`]
    /// ([`crate::WordFormat::pages`]); `None` where neither is given, or for a document that
    /// is not a Word document.
    ///
    /// # Errors
    /// `NotOpen` where no document `id` is open.
    fn pages(&self, id: &DocId) -> Result<Option<Arc<EnginePages>>, SessionError>;

    /// Lay the document `id` out with `pages` from now on (`None`: with the session's): its
    /// engine is made again when next needed. A Word document only; another kind keeps none.
    ///
    /// # Errors
    /// `NotOpen` where no document `id` is open.
    fn set_pages(&self, id: &DocId, pages: Option<SessionPages>) -> Result<(), SessionError>;

    /// What the document `id` keeps in the session's memory budget now, bytes: its page
    /// engine's accounts (what it holds of the version in use, and its pages' paint); 0
    /// where it has no engine.
    ///
    /// # Errors
    /// `NotOpen` where no document `id` is open.
    fn document_memory(&self, id: &DocId) -> Result<usize, SessionError>;
}

impl WordPages for Session {
    fn pages(&self, id: &DocId) -> Result<Option<Arc<EnginePages>>, SessionError> {
        let doc = self.format_doc(id)?;
        Ok(Word::of(&*doc).and_then(Word::engine))
    }

    fn set_pages(&self, id: &DocId, pages: Option<SessionPages>) -> Result<(), SessionError> {
        let doc = self.format_doc(id)?;
        if let Some(word) = Word::of(&*doc) {
            let mut held = word.pages.0.lock().unwrap_or_else(PoisonError::into_inner);
            *held = Held {
                own: pages,
                ..Held::default()
            };
        }
        Ok(())
    }

    fn document_memory(&self, id: &DocId) -> Result<usize, SessionError> {
        let doc = self.format_doc(id)?;
        let Some(word) = Word::of(&*doc) else {
            return Ok(0);
        };
        let held = word.pages.0.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(held.accounts.iter().map(|a| a.used()).sum())
    }
}

impl Word {
    /// Its page engine: made on first use from what its host set it to be laid out with,
    /// else the session's default; `None` where neither is given.
    pub(crate) fn engine(&self) -> Option<Arc<EnginePages>> {
        let mut held = self.pages.0.lock().unwrap_or_else(PoisonError::into_inner);
        if held.engine.is_none() {
            let made = held
                .own
                .as_ref()
                .or(self.default_pages.as_ref())
                .map(SessionPages::engine);
            if let Some((engine, accounts)) = made {
                held.engine = Some(engine);
                held.accounts = accounts.into();
            }
        }
        held.engine.clone()
    }

    /// The session's own layout of the document, for a call whose host gives none.
    pub(crate) fn own_pages(&self) -> Option<Arc<dyn WordLayout>> {
        let engine = self.engine()?;
        Some(engine as Arc<dyn WordLayout>)
    }
}
