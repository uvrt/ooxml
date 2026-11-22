//! Owns: what a Word document keeps for its live view ([`LiveViews`]): the versions the
//! views were given, by token -- each one's layout ([`LaidVersion`]), its snapshot (its
//! package and ids) and its pages as a view lists them -- at most [`VIEW_TOKENS`], the least
//! recently used let go first, each layout's weight reserved from the session's memory
//! budget (one the budget cannot give room is kept with the newest, reserving nothing).
//!
//! Must not: lay out or paint (the session's view does, through the host's engine), or hold
//! its lock while anything but its list is read or changed.

use super::ViewPage;
use ooxml_base::CacheBudget;
use ooxml_store::Snapshot;
use ooxml_word_edit::WordDoc;
use ooxml_word_pages::LaidVersion;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

/// Versions a document keeps for its views (docx-agent's `VIEW_TOKENS`): `since` and a
/// page's token name one of these.
pub const VIEW_TOKENS: usize = 4;

/// One version a view was given.
pub(crate) struct Kept {
    /// Its token.
    pub(crate) token: String,
    /// Its layout.
    pub(crate) version: Arc<LaidVersion>,
    /// The version itself: its package (laid out again where the engine no longer paints
    /// it) and its ids (a page's SVG is tagged with).
    pub(crate) at: Snapshot<WordDoc>,
    /// Its pages, as a view lists them.
    pub(crate) pages: Arc<[ViewPage]>,
    /// What its layout reserved of the budget.
    reserved: usize,
}

impl Kept {
    /// The version `at`, laid out as `version`, its pages `pages`, named `token`.
    pub(crate) fn new(token: String, version: Arc<LaidVersion>, at: Snapshot<WordDoc>, pages: Vec<ViewPage>) -> Kept {
        Kept {
            token,
            version,
            at,
            pages: pages.into(),
            reserved: 0,
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

    /// The version `token` names, made the most recently used.
    pub(crate) fn get(&self, token: &str) -> Option<Arc<Kept>> {
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        let at = kept.iter().position(|k| k.token == token)?;
        let found = kept.remove(at)?;
        kept.push_back(Arc::clone(&found));
        Some(found)
    }

    /// Keep `made` as the most recently used, letting the least recently used go past
    /// [`VIEW_TOKENS`] and while the budget has no room for its layout; one already kept
    /// under its token (another call's, made meanwhile) is kept instead.
    pub(crate) fn keep(&self, mut made: Kept) -> Arc<Kept> {
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = kept.iter().position(|k| k.token == made.token) {
            if let Some(found) = kept.remove(at) {
                kept.push_back(Arc::clone(&found));
                return found;
            }
        }
        while kept.len() >= VIEW_TOKENS {
            self.let_go(&mut kept);
        }
        let weight = made.version.weight();
        loop {
            if self.budget.reserve(weight) {
                made.reserved = weight;
                break;
            }
            if kept.is_empty() {
                break;
            }
            self.let_go(&mut kept);
        }
        let made = Arc::new(made);
        kept.push_back(Arc::clone(&made));
        made
    }

    fn let_go(&self, kept: &mut VecDeque<Arc<Kept>>) {
        if let Some(gone) = kept.pop_front() {
            self.budget.release(gone.reserved);
        }
    }
}

impl Drop for LiveViews {
    fn drop(&mut self) {
        let kept = self.kept.get_mut().unwrap_or_else(PoisonError::into_inner);
        while let Some(gone) = kept.pop_front() {
            self.budget.release(gone.reserved);
        }
    }
}
