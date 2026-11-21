//! Owns: [`CallEditor`], a document's editor as one call's handler is handed it: each edit
//! runs through the editor as before (one command, one history step), and the version the
//! call made is kept ([`Made`]) for the host -- the call's own, whatever other calls commit
//! to the document meanwhile (the head after the call may be theirs). A call commits at most
//! once, so it is one step and one state number: a handler with several commands runs them
//! as one transaction (the session checks it in debug builds).
//!
//! Must not: change how an edit runs, or outlive the call.

use ooxml_store::{Caller, Command, Seq, Snapshot, Store};
use ooxml_word_edit::touched::Touched;
use ooxml_word_edit::validate::{Delta, Problem};
use ooxml_word_edit::{EditError, Edited, Editor, IdsDelta, WordDoc};
use std::sync::{Mutex, PoisonError};

/// The versions one call made: its first edit's and its last's (the same for one edit),
/// with what its last edit touched and the validation delta of the version it made (the
/// editor works both out with each edit).
#[derive(Debug, Clone)]
pub struct Made {
    /// The version the call's first changing edit made.
    pub first: Snapshot<WordDoc>,
    /// The version its last changing edit made: the call's step ends there.
    pub last: Snapshot<WordDoc>,
    /// What the last edit touched (from the version it was applied to).
    pub touched: Vec<Touched>,
    /// The validation delta of `last`.
    pub validation: Delta,
    /// The ids each changing edit removed and brought, by the version it made.
    pub ids: Vec<(Seq, IdsDelta)>,
}

/// The document's editor for one call: its edits, and the versions they made.
pub struct CallEditor<'a> {
    editor: &'a Editor,
    made: Mutex<Option<Made>>,
    /// The stand-ins its edits drew that were drawn as other values where they landed.
    moved: Mutex<Vec<(String, String)>>,
}

impl<'a> CallEditor<'a> {
    /// `editor`, for one call.
    #[must_use]
    pub fn new(editor: &'a Editor) -> Self {
        Self {
            editor,
            made: Mutex::new(None),
            moved: Mutex::new(Vec::new()),
        }
    }

    /// Run `command` as `caller` through the editor ([`Editor::run`]); a version it made is
    /// the call's.
    ///
    /// # Errors
    /// The edit's.
    pub fn run<C>(&self, caller: &Caller, command: &C) -> Result<Edited<C::Out>, EditError>
    where
        C: Command<WordDoc, Error = EditError>,
    {
        let edited = self.editor.run(caller, command)?;
        if edited.drawn.moved().next().is_some() {
            let mut moved = self.moved.lock().unwrap_or_else(PoisonError::into_inner);
            moved.extend(edited.drawn.moved().map(|(s, v)| (s.to_owned(), v.to_owned())));
        }
        if edited.changed {
            let mut made = self.made.lock().unwrap_or_else(PoisonError::into_inner);
            let (first, mut ids) = made
                .take()
                .map_or_else(|| (edited.version.clone(), Vec::new()), |m| (m.first, m.ids));
            ids.push((edited.version.seq(), edited.ids.clone()));
            *made = Some(Made {
                first,
                last: edited.version.clone(),
                touched: edited.touched.clone(),
                validation: edited.validation.clone(),
                ids,
            });
        }
        Ok(edited)
    }

    /// The document's versions.
    #[must_use]
    pub fn store(&self) -> &Store<WordDoc> {
        self.editor.store()
    }

    /// The problems the document opened with.
    #[must_use]
    pub fn baseline(&self) -> &[Problem] {
        self.editor.baseline()
    }

    /// The stand-ins the call's edits drew that landed as other values: `(stand-in, value)`
    /// ([`crate::resolve_drawn`] names them so in the reply).
    #[must_use]
    pub fn moved(&self) -> Vec<(String, String)> {
        self.moved.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The versions the call's edits made so far; `None` while none changed the document.
    #[must_use]
    pub fn made(&self) -> Option<Made> {
        self.made.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}
