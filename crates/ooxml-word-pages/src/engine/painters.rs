//! Owns: what a version of the engine's generation is painted and measured with
//! ([`Painter`]): the model that resolves its blocks' formats and the layout whose
//! measures its stories (headers, footers, notes, text boxes) are laid out with -- both of
//! the model's context (its styles, numbering, settings, theme) the version was laid out
//! in. The generation's own model and layout paint the versions of its current context;
//! a version of a context the generation has left is painted by a painter of that
//! context's own ([`Contexts`]), made when first asked for and kept while a version of
//! that context is held ([`ContextMark`]), so a version laid out before a style edit is
//! drawn with its own styles, not the edit's.
//!
//! Must not: open a version (a past context's painter only resolves against the context
//! its versions carry), or let a resolution or measure made in one context be read in
//! another (each painter's memos are of one context).

use super::parts::new_layout;
use super::{Generation, KEY};
use crate::fonts::DocumentFonts;
use crate::version::LaidVersion;
use ooxml_base::Locale;
use ooxml_derive::CacheBudget;
use ooxml_word_layout::WordLayout;
use ooxml_word_model::Model;
use std::sync::{Arc, Weak};

/// What every version laid out in one context holds: while one is held, that context's
/// painter is kept.
#[derive(Debug, Default)]
pub(crate) struct ContextMark;

/// What a version is painted and measured with: its context's model and layout.
pub(super) struct Painter<'g> {
    pub(super) model: &'g mut Model,
    pub(super) layout: &'g mut WordLayout,
}

/// A context the generation has left, painted on its own.
struct Past {
    context: u64,
    held: Weak<ContextMark>,
    model: Model,
    layout: WordLayout,
}

/// The generation's contexts: the current one's mark, and the painters of the contexts it
/// left whose versions are still held.
pub(super) struct Contexts {
    current: Arc<ContextMark>,
    past: Vec<Past>,
}

impl Contexts {
    pub(super) fn new() -> Contexts {
        Contexts {
            current: Arc::new(ContextMark),
            past: Vec::new(),
        }
    }

    /// The mark a version laid out in the current context holds.
    pub(super) fn mark(&self) -> Arc<ContextMark> {
        Arc::clone(&self.current)
    }

    /// The context changed: the versions laid out before hold the old mark, the next ones
    /// a new one.
    pub(super) fn leave(&mut self) {
        self.current = Arc::new(ContextMark);
        self.sweep();
    }

    /// Let go of the painters of contexts no version of is held any more.
    pub(super) fn sweep(&mut self) {
        self.past.retain(|past| past.held.strong_count() > 0);
    }

    /// The painter of `version`, of a context the generation has left: made (a model of
    /// its own, a layout of the generation's `fonts`) when first asked for.
    fn past(
        &mut self,
        version: &LaidVersion,
        (fonts, locale, budget): (&DocumentFonts, &Locale, &Arc<dyn CacheBudget>),
    ) -> Option<Painter<'_>> {
        self.sweep();
        if !self.past.iter().any(|past| past.context == version.context) {
            self.past.push(Past {
                context: version.context,
                held: Arc::downgrade(&version.painted_in),
                model: Model::new(Arc::clone(budget), KEY),
                layout: new_layout(fonts, locale, budget),
            });
        }
        let past = self.past.iter_mut().find(|past| past.context == version.context)?;
        Some(Painter {
            model: &mut past.model,
            layout: &mut past.layout,
        })
    }
}

impl Generation {
    /// What `version` is painted and measured with: the generation's own model and layout
    /// where it was laid out in the current context, else its own context's painter;
    /// `None` where it is of another generation.
    pub(super) fn painter(
        &mut self,
        version: &LaidVersion,
        locale: &Locale,
        budget: &Arc<dyn CacheBudget>,
    ) -> Option<Painter<'_>> {
        if version.generation != self.number {
            return None;
        }
        if version.context == self.context {
            return Some(Painter {
                model: &mut self.model,
                layout: &mut self.layout,
            });
        }
        self.contexts.past(version, (&self.fonts, locale, budget))
    }
}
