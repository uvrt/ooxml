//! Owns: [`CallEditor`], a document's editor as one call's handler is handed it: each edit
//! runs through the editor as before (one command, one history step), and the versions the
//! call made are kept ([`Made`]) for the host -- the call's own versions, whatever other
//! calls commit to the document meanwhile (the head after the call may be theirs).
//!
//! Must not: change how an edit runs, or outlive the call.

use ooxml_store::{Caller, Command, Snapshot, Store};
use ooxml_word_edit::validate::Problem;
use ooxml_word_edit::{EditError, Edited, Editor, WordDoc};
use std::sync::{Mutex, PoisonError};

/// The versions one call made: its first edit's and its last's (the same for one edit).
#[derive(Debug, Clone)]
pub struct Made {
    /// The version the call's first changing edit made.
    pub first: Snapshot<WordDoc>,
    /// The version its last changing edit made: the call's step ends there.
    pub last: Snapshot<WordDoc>,
}

/// The document's editor for one call: its edits, and the versions they made.
pub struct CallEditor<'a> {
    editor: &'a Editor,
    made: Mutex<Option<Made>>,
}

impl<'a> CallEditor<'a> {
    /// `editor`, for one call.
    #[must_use]
    pub fn new(editor: &'a Editor) -> Self {
        Self {
            editor,
            made: Mutex::new(None),
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
        if edited.changed {
            let mut made = self.made.lock().unwrap_or_else(PoisonError::into_inner);
            let first = made.take().map_or_else(|| edited.version.clone(), |m| m.first);
            *made = Some(Made {
                first,
                last: edited.version.clone(),
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

    /// The versions the call's edits made so far; `None` while none changed the document.
    #[must_use]
    pub fn made(&self) -> Option<Made> {
        self.made.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}
