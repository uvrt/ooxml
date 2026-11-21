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

use crate::retired::Made;
use crate::session::Opened;
use crate::{CallContext, DocId, Session, SessionError};
use ooxml_store::{DocModel, Error, HistoryOutcome, Listed, StepInfo, Store, StoreError};
use ooxml_word_edit::WordDoc;

/// What an undo or redo through the session did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Undone {
    /// How many calls it took back or brought back (0: none was there).
    pub count: usize,
    /// The state number of the document's head after it ([`Opened::number`]).
    pub version: u64,
}

/// One step of a document's history in the tool layer's state numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberedStep {
    /// The state it was made on.
    pub before: u64,
    /// The state it made.
    pub after: u64,
    /// The version it made (the store's sequence number).
    pub seq: u64,
    /// Who made it (the caller's key; `None`: the session's own).
    pub caller: Option<String>,
}

/// A document's history in the tool layer's state numbers, read at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Numbers {
    /// The highest state number given: the next step's is one more.
    pub highest: u64,
    /// The steps undo can reach, oldest first.
    pub steps: Vec<NumberedStep>,
    /// The steps plain redo can bring back, the next one last.
    pub redo: Vec<NumberedStep>,
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

    /// The document's history in state numbers: the highest given, the steps undo can
    /// reach and those redo can bring back.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn numbers(&self, id: &DocId) -> Result<Numbers, SessionError> {
        let document = self.document(id)?;
        Ok(match &document.held {
            crate::document::Held::Package(store) => numbers_of(store.listed()),
            crate::document::Held::Word(word) => numbers_of(word.editor.store().listed()),
        })
    }

    /// The highest state number given on the document: the next step's is one more.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn highest(&self, id: &DocId) -> Result<u64, SessionError> {
        let document = self.document(id)?;
        Ok(match &document.held {
            crate::document::Held::Package(store) => store.highest(),
            crate::document::Held::Word(word) => word.editor.store().highest(),
        })
    }

    /// Go on with the state numbers the host gave the document before it was opened again:
    /// its head is the state `state`, the next step's number one more than `highest`. Only
    /// before the document's first step.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; [`SessionError::Store`] when the numbers are refused (a
    /// state above the highest, or a step made already).
    pub fn continue_numbers(&self, id: &DocId, state: u64, highest: u64) -> Result<(), SessionError> {
        let document = self.document(id)?;
        let done = match &document.held {
            crate::document::Held::Package(store) => store.continue_numbers(state, highest),
            crate::document::Held::Word(word) => word.editor.store().continue_numbers(state, highest),
        };
        done.map_err(|e| SessionError::Store(StoreError::Counters(e)))
    }

    /// The document's state numbered `number` ([`Opened::number`]) its head reaches: the
    /// head, else the newest such state undo or redo can bring back; `None` when none is kept.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn numbered(&self, id: &DocId, number: u64) -> Result<Option<Opened>, SessionError> {
        let document = self.document(id)?;
        let at = match &document.held {
            crate::document::Held::Package(store) => store.numbered(number).map(crate::document::At::Package),
            crate::document::Held::Word(word) => word.editor.store().numbered(number).map(crate::document::At::Word),
        };
        Ok(at.map(|at| Self::opened(id, &document, at)))
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
            crate::document::Held::Package(store) => run(store, steps, caller, direction, &mut |_| Ok(()))?,
            crate::document::Held::Word(word) => {
                // what each undo or redo removed and brought back, in the document's retired ids
                let mut note = |done: &HistoryOutcome<WordDoc>| -> Result<(), SessionError> {
                    let delta = ooxml_word_edit::ids_delta(done.previous.doc(), done.version.doc())?;
                    let made = Made {
                        seq: done.version.seq().0,
                        caller,
                        tool: None,
                        history: true,
                    };
                    word.retired.note(&made, &delta);
                    Ok(())
                };
                run(word.editor.store(), steps, caller, direction, &mut note)?
            }
        };
        Ok(Undone {
            count,
            version: document.head().number(),
        })
    }
}

fn numbers_of<K: Ord>(listed: Listed<K>) -> Numbers {
    let step = |s: StepInfo<K>| NumberedStep {
        before: s.from_number,
        after: s.number,
        seq: s.step.0,
        caller: s.caller.as_deref().map(str::to_owned),
    };
    Numbers {
        highest: listed.highest,
        steps: listed.steps.into_iter().map(step).collect(),
        redo: listed.redo.into_iter().map(step).collect(),
    }
}

/// Up to `steps` undos or redos on `store`, as `caller` (`None`: the session's own).
fn run<D: DocModel>(
    store: &Store<D>,
    steps: usize,
    caller: Option<&str>,
    direction: Direction,
    note: &mut dyn FnMut(&HistoryOutcome<D>) -> Result<(), SessionError>,
) -> Result<usize, SessionError>
where
    SessionError: From<Error<D>>,
{
    let mut done = 0;
    while done < steps {
        let outcome = match (direction, caller) {
            (Direction::Undo, None) => store.undo(),
            (Direction::Redo, None) => store.redo(),
            (Direction::Undo, Some(caller)) => match own_undo(store, caller) {
                Ok(Some(outcome)) => Ok(outcome),
                Ok(None) => break,
                Err(error) if done == 0 => return Err(error),
                Err(_) => break,
            },
            (Direction::Redo, Some(caller)) => {
                let next = store.redoable().pop();
                if next.and_then(|step| step.caller).as_deref() != Some(caller) {
                    break;
                }
                store.redo()
            }
        };
        match outcome {
            Ok(outcome) => {
                note(&outcome)?;
                done += 1;
            }
            Err(StoreError::NothingToUndo | StoreError::NothingToRedo) => break,
            Err(error) if done == 0 => return Err(error.into()),
            Err(_) => break,
        }
    }
    Ok(done)
}

/// Undo `caller`'s latest step when it is the document's latest; `None` when it has none.
fn own_undo<D: DocModel>(store: &Store<D>, caller: &str) -> Result<Option<HistoryOutcome<D>>, SessionError>
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
    Ok(Some(store.undo()?))
}
