//! Owns: what a Word document keeps for its live view ([`LiveViews`]): the versions the
//! views were given, by token -- each one's layout ([`LaidVersion`]), its snapshot (its
//! package and ids), its pages as a view lists them, and, for a version the host's engine
//! no longer paints, the spare engine's layout of it ([`Spare`]) -- at most [`VIEW_TOKENS`],
//! the least recently used let go first. Every layout kept is reserved from the session's
//! memory budget: one the budget has no room for lets the least recently used others go
//! until it fits, and where it still does not fit it is not kept (the version's snapshot
//! and pages are: its token is stale to a later view's `since`, and its pages are laid out
//! again on demand).
//!
//! Must not: lay out or paint (the session's view does, through the host's engine), hold
//! its lock while anything but its list is read or changed, or keep a layout the budget
//! gave no room for.

use super::ViewPage;
use ooxml_base::CacheBudget;
use ooxml_store::Snapshot;
use ooxml_tools_word::EnginePages;
use ooxml_word_edit::WordDoc;
use ooxml_word_pages::LaidVersion;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Versions a document keeps for its views (docx-agent's `VIEW_TOKENS`): `since` and a
/// page's token name one of these.
pub const VIEW_TOKENS: usize = 4;

/// A spare engine's layout of a kept version the host's engine no longer paints: its
/// pages are painted from it, laid out once.
#[derive(Clone)]
pub(crate) struct Spare {
    /// The spare engine, its last layout `version`.
    pub(crate) engine: Arc<EnginePages>,
    /// Its layout of the version.
    pub(crate) version: Arc<LaidVersion>,
}

/// A layout kept, and what it reserved of the budget.
struct Reserved<T> {
    held: T,
    bytes: usize,
}

/// One version a view was given.
pub(crate) struct Kept {
    /// Its token.
    pub(crate) token: String,
    /// The version itself: its package (laid out again where the engine no longer paints
    /// it) and its ids (a page's SVG is tagged with).
    pub(crate) at: Snapshot<WordDoc>,
    /// Its pages, as a view lists them.
    pub(crate) pages: Arc<[ViewPage]>,
    /// Its layout by the host's engine, while the budget has room for it.
    laid: Option<Reserved<Arc<LaidVersion>>>,
    /// A spare engine's layout of it, once one was made and the budget had room for it.
    spare: Mutex<Option<Reserved<Spare>>>,
}

impl Kept {
    /// The version `at`, laid out as `version`, its pages `pages`, named `token`.
    pub(crate) fn new(token: String, version: Arc<LaidVersion>, at: Snapshot<WordDoc>, pages: Vec<ViewPage>) -> Kept {
        Kept {
            token,
            at,
            pages: pages.into(),
            laid: Some(Reserved {
                held: version,
                bytes: 0,
            }),
            spare: Mutex::new(None),
        }
    }

    /// Its layout by the host's engine; `None` where the budget had no room for it.
    pub(crate) fn version(&self) -> Option<&Arc<LaidVersion>> {
        self.laid.as_ref().map(|laid| &laid.held)
    }

    /// The spare engine's layout of it, where one is kept.
    pub(crate) fn spare(&self) -> Option<Spare> {
        self.spare_slot().as_ref().map(|spare| spare.held.clone())
    }

    fn spare_slot(&self) -> MutexGuard<'_, Option<Reserved<Spare>>> {
        self.spare.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What it reserved of the budget, given back as it is let go (its spare layout
    /// dropped: a caller painting from it holds its own).
    fn release(&self, budget: &dyn CacheBudget) {
        if let Some(laid) = &self.laid {
            budget.release(laid.bytes);
        }
        if let Some(spare) = self.spare_slot().take() {
            budget.release(spare.bytes);
        }
    }
}

/// A document's kept versions, the most recently used last.
pub(crate) struct LiveViews {
    budget: Arc<dyn CacheBudget>,
    kept: Mutex<VecDeque<Arc<Kept>>>,
}

impl LiveViews {
    /// None kept yet; layouts reserved from `budget`.
    pub(crate) fn new(budget: Arc<dyn CacheBudget>) -> LiveViews {
        LiveViews {
            budget,
            kept: Mutex::new(VecDeque::new()),
        }
    }

    fn list(&self) -> MutexGuard<'_, VecDeque<Arc<Kept>>> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The version `token` names, made the most recently used.
    pub(crate) fn get(&self, token: &str) -> Option<Arc<Kept>> {
        let mut kept = self.list();
        let at = kept.iter().position(|k| k.token == token)?;
        let found = kept.remove(at)?;
        kept.push_back(Arc::clone(&found));
        Some(found)
    }

    /// Keep `made` as the most recently used, letting the least recently used go past
    /// [`VIEW_TOKENS`] and while the budget has no room for its layout; its layout is not
    /// kept where the budget has no room for it with every other let go. One already kept
    /// under its token (another call's, made meanwhile) is kept instead.
    pub(crate) fn keep(&self, mut made: Kept) -> Arc<Kept> {
        let mut kept = self.list();
        if let Some(at) = kept.iter().position(|k| k.token == made.token) {
            if let Some(found) = kept.remove(at) {
                kept.push_back(Arc::clone(&found));
                return found;
            }
        }
        while kept.len() >= VIEW_TOKENS {
            self.let_go(&mut kept, None);
        }
        if let Some(laid) = made.laid.take() {
            let weight = laid.held.weight();
            made.laid = self.make_room(&mut kept, weight, None).then_some(Reserved {
                held: laid.held,
                bytes: weight,
            });
        }
        let made = Arc::new(made);
        kept.push_back(Arc::clone(&made));
        made
    }

    /// Keep `spare` as `kept`'s spare layout where it is still kept, has none yet, and the
    /// budget has room for it once the least recently used others are let go.
    pub(crate) fn keep_spare(&self, kept: &Arc<Kept>, spare: &Spare) {
        let mut list = self.list();
        if !list.iter().any(|k| Arc::ptr_eq(k, kept)) || kept.spare_slot().is_some() {
            return;
        }
        let weight = spare.version.weight();
        if self.make_room(&mut list, weight, Some(kept)) {
            *kept.spare_slot() = Some(Reserved {
                held: spare.clone(),
                bytes: weight,
            });
        }
    }

    /// Reserve `weight`, letting the least recently used go (`keep` never) until the budget
    /// has room for it; `false` where it has none with every other let go.
    fn make_room(&self, list: &mut VecDeque<Arc<Kept>>, weight: usize, keep: Option<&Arc<Kept>>) -> bool {
        loop {
            if self.budget.reserve(weight) {
                return true;
            }
            if !self.let_go(list, keep) {
                return false;
            }
        }
    }

    /// Let the least recently used go for another's room (the changes feed's, design
    /// section 4: older views' layouts go first), never the most recent; `false` where
    /// there is no older one.
    pub(crate) fn let_go_older(&self) -> bool {
        let mut list = self.list();
        if list.len() < 2 {
            return false;
        }
        self.let_go(&mut list, None)
    }

    /// Let the least recently used go but `keep`; `false` where there is none.
    fn let_go(&self, list: &mut VecDeque<Arc<Kept>>, keep: Option<&Arc<Kept>>) -> bool {
        let at = list.iter().position(|k| keep.is_none_or(|keep| !Arc::ptr_eq(k, keep)));
        match at.and_then(|at| list.remove(at)) {
            Some(gone) => {
                gone.release(&*self.budget);
                true
            }
            None => false,
        }
    }
}

impl Drop for LiveViews {
    fn drop(&mut self) {
        let kept = self.kept.get_mut().unwrap_or_else(PoisonError::into_inner);
        while let Some(gone) = kept.pop_front() {
            gone.release(&*self.budget);
        }
    }
}
