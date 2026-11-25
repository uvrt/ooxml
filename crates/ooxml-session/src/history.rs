//! Owns: undo and redo of a document's calls through the session ([`Session::undo`],
//! [`Session::redo`]), as docx-agent's session does them for a Word document: the session's
//! own undo takes back the latest calls, one per step; a caller's own undo takes back only
//! its own latest call, and only while it is the document's latest change (a Word body is
//! one scope every change shares: a later change by anyone else makes it
//! [`SessionError::Entangled`], naming the later steps); a caller's own redo brings back its
//! own latest undo while it is the next one redo would bring back, else
//! [`SessionError::RedoNotNext`] (another caller undid a change since). The check and the
//! undo or redo are one step of the store's ([`ooxml_store::Store::undo_newest`]), so no commit lands
//! between them ([`crate::Stored`] runs it). Ids given stay given (the store keeps the ledger
//! across both); the document's format is told what each step brought back.
//!
//! Must not: plan an undo itself (the store does), or say anything in a tool's words.

use crate::session::Opened;
use crate::{CallContext, DocId, Session, SessionError};
use ooxml_store::{Listed, StepInfo};

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

/// Which way a history run goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Take steps back.
    Undo,
    /// Bring undone steps back.
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
        Ok(self.document(id)?.stored().numbers())
    }

    /// The highest state number given on the document: the next step's is one more.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn highest(&self, id: &DocId) -> Result<u64, SessionError> {
        Ok(self.document(id)?.stored().highest())
    }

    /// Go on with the state numbers the host gave the document before it was opened again:
    /// its head is the state `state`, the next step's number one more than `highest`. Only
    /// before the document's first step.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; [`SessionError::Store`] when the numbers are refused (a
    /// state above the highest, or a step made already).
    pub fn continue_numbers(&self, id: &DocId, state: u64, highest: u64) -> Result<(), SessionError> {
        self.document(id)?.stored().continue_numbers(state, highest)
    }

    /// The document's state numbered `number` ([`Opened::number`]) its head reaches: the
    /// head, else the newest such state undo or redo can bring back; `None` when none is kept.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn numbered(&self, id: &DocId, number: u64) -> Result<Option<Opened>, SessionError> {
        let document = self.document(id)?;
        let at = document.stored().numbered(number);
        Ok(at.map(|at| Self::opened(id, &document, at)))
    }

    pub(crate) fn history(
        &self,
        id: &DocId,
        steps: usize,
        context: &CallContext,
        direction: Direction,
    ) -> Result<Undone, SessionError> {
        let document = self.document(id)?;
        let count = document.stored().history(steps, context.caller.name(), direction)?;
        Ok(Undone {
            count,
            version: document.head().number(),
        })
    }
}

pub(crate) fn numbers_of<K: Ord>(listed: Listed<K>) -> Numbers {
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
