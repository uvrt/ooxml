//! Owns: undo and redo of a document's calls through the session ([`Session::undo`],
//! [`Session::redo`]), as docx-agent's session does them for a Word document: the session's
//! own undo takes back the latest calls, one per step; a caller's own undo takes back only
//! its own latest call, and only while it is the document's latest change (a Word body is
//! one scope every change shares: a later change by anyone else makes it
//! [`SessionError::Entangled`], naming the later changes and who made them); a caller's own
//! redo brings back its own undone call while it is the next one redo would bring back.
//! Ids given stay given (the store keeps the ledger across both).
//!
//! Must not: plan an undo itself (the store does), or say anything in a tool's words.

use crate::{CallContext, DocId, Session, SessionError};
use ooxml_store::{DocModel, Error, Store, StoreError};

/// What an undo or redo through the session did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Undone {
    /// How many calls it took back or brought back (0: none was there).
    pub count: usize,
    /// The document's version after it.
    pub version: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Undo,
    Redo,
}

impl Session {
    /// Undo up to `steps` calls on `id`: the latest ones, or with a named caller in
    /// `context`, that caller's own (see the module's rule).
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; [`SessionError::Entangled`] when the caller's latest call
    /// is not the document's latest change and nothing was undone yet.
    pub fn undo(&self, id: &DocId, steps: usize, context: &CallContext) -> Result<Undone, SessionError> {
        self.history(id, steps, context, Direction::Undo)
    }

    /// Redo up to `steps` undone calls on `id` (with a named caller, its own).
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn redo(&self, id: &DocId, steps: usize, context: &CallContext) -> Result<Undone, SessionError> {
        self.history(id, steps, context, Direction::Redo)
    }

    fn history(
        &self,
        id: &DocId,
        steps: usize,
        context: &CallContext,
        direction: Direction,
    ) -> Result<Undone, SessionError> {
        let document = self.document(id)?;
        let caller = context.caller.name();
        let count = match &document.held {
            crate::document::Held::Package(store) => run(store, steps, caller, direction)?,
            crate::document::Held::Word(word) => run(word.editor.store(), steps, caller, direction)?,
        };
        Ok(Undone {
            count,
            version: document.head().seq(),
        })
    }
}

/// Up to `steps` undos or redos on `store`, as `caller` (`None`: the session's own).
fn run<D: DocModel>(
    store: &Store<D>,
    steps: usize,
    caller: Option<&str>,
    direction: Direction,
) -> Result<usize, SessionError>
where
    SessionError: From<Error<D>>,
{
    let mut done = 0;
    while done < steps {
        let outcome = match (direction, caller) {
            (Direction::Undo, None) => store.undo().map(drop),
            (Direction::Redo, None) => store.redo().map(drop),
            (Direction::Undo, Some(caller)) => match own_undo(store, caller) {
                Ok(Some(())) => Ok(()),
                Ok(None) => break,
                Err(error) if done == 0 => return Err(error),
                Err(_) => break,
            },
            (Direction::Redo, Some(caller)) => {
                let next = store.redoable().pop();
                if next.and_then(|step| step.caller).as_deref() != Some(caller) {
                    break;
                }
                store.redo().map(drop)
            }
        };
        match outcome {
            Ok(()) => done += 1,
            Err(StoreError::NothingToUndo | StoreError::NothingToRedo) => break,
            Err(error) if done == 0 => return Err(error.into()),
            Err(_) => break,
        }
    }
    Ok(done)
}

/// Undo `caller`'s latest step when it is the document's latest; `None` when it has none.
fn own_undo<D: DocModel>(store: &Store<D>, caller: &str) -> Result<Option<()>, SessionError>
where
    SessionError: From<Error<D>>,
{
    let steps = store.steps();
    let Some(index) = steps.iter().rposition(|s| s.caller.as_deref() == Some(caller)) else {
        return Ok(None);
    };
    let later: Vec<(u64, Option<String>)> = steps
        .iter()
        .skip(index + 1)
        .map(|s| (s.step.0, s.caller.as_deref().map(str::to_owned)))
        .collect();
    if let Some(step) = steps.get(index).filter(|_| !later.is_empty()) {
        return Err(SessionError::Entangled {
            step: step.step.0,
            later,
        });
    }
    store.undo()?;
    Ok(Some(()))
}
