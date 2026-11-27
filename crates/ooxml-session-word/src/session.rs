//! Owns: a session's Word documents as their host reads and opens them ([`WordSession`]):
//! opening one with the ids it had before it was saved, or with the id state saved beside
//! its bytes; the save gate's comparison; what its last check saw; and which errors are a
//! Word document's refusals ([`refuses_state`], [`not_word`]).
//!
//! Must not: hold documents (the session does) or dispatch a call.

use crate::word::Word;
use ooxml_opc::Package;
use ooxml_session::{DocId, DocKind, Opened, Session, SessionError};
use ooxml_word_edit::validate::Delta;
use ooxml_word_edit::{EarlierIds, EditError, WordDoc};
use ooxml_word_model::ids::{HeldId, LoadedState};
use std::sync::Arc;

/// A session's Word documents as their host reads and opens them.
pub trait WordSession {
    /// Open a Word document from its bytes as `name`, with the ids it had before it was
    /// saved (the host's state of it, [`EarlierIds`]): it never gives a retired id again;
    /// another `kind` as [`Session::open`] opens it.
    ///
    /// # Errors
    /// As [`Session::open`].
    fn open_with(
        &self,
        bytes: Arc<[u8]>,
        kind: DocKind,
        name: &str,
        earlier: &EarlierIds,
    ) -> Result<DocId, SessionError>;

    /// A Word document opened from its bytes as `name`, with the id state saved beside them
    /// (`docx-ids.1.`, docx-agent's `Document.id_state`): its ids are read against its parts
    /// as they are parsed, so it never gives an id it had before it was saved again, and each
    /// id the state retired is the session's record (who retired it, in the host's words).
    /// Returns its id and the state's `(version, last)`: the version saved and the highest
    /// its history gave, for the host's numbers to go on from.
    ///
    /// # Errors
    /// As [`Session::open`]; [`SessionError::Format`] with [`EditError::IdState`] when the
    /// state is not these bytes' ([`refuses_state`]).
    fn open_state(&self, bytes: Arc<[u8]>, name: &str, state: &str) -> Result<(DocId, (i64, i64)), SessionError>;

    /// What the save gate compares for a Word document at its head: its problems now
    /// against those it opened with (`None` for another kind).
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the edit core's when a version cannot be read.
    fn validation(&self, id: &DocId) -> Result<Option<Delta>, SessionError>;

    /// [`WordSession::validation`] of the version `at` (one a call made, say) instead of
    /// the head.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the edit core's when the version cannot be read.
    fn validation_at(&self, at: &Opened) -> Result<Option<Delta>, SessionError>;

    /// What the Word document's last check with `reflow` saw (`since_last_check` is
    /// measured from it), read and kept by a host's own checks too; `None` for another kind.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    fn last_check(&self, id: &DocId) -> Result<Option<Arc<ooxml_tools_word::LastCheck>>, SessionError>;
}

impl WordSession for Session {
    fn open_with(
        &self,
        bytes: Arc<[u8]>,
        kind: DocKind,
        name: &str,
        earlier: &EarlierIds,
    ) -> Result<DocId, SessionError> {
        if kind != DocKind::Docx {
            return self.open(bytes, kind, name);
        }
        let package = Package::open(bytes, &self.context().limits)?;
        let word = Word::open(package, &self.open_env(), earlier, &format_of(self))?;
        Ok(self.insert(DocKind::Docx, name, Arc::new(word)))
    }

    fn open_state(&self, bytes: Arc<[u8]>, name: &str, state: &str) -> Result<(DocId, (i64, i64)), SessionError> {
        let package = Package::open(bytes, &self.context().limits)?;
        let (word, numbers) = Word::open_seeded(package, &self.open_env(), state, &format_of(self))?;
        let id = self.insert(DocKind::Docx, name, Arc::new(word));
        Ok((id, numbers))
    }

    fn validation(&self, id: &DocId) -> Result<Option<Delta>, SessionError> {
        let doc = self.format_doc(id)?;
        let head = doc.stored().head();
        match (Word::of(&*doc), head.snapshot::<WordDoc>()) {
            (Some(word), Some(version)) => Ok(Some(word.validation(version)?)),
            _ => Ok(None),
        }
    }

    fn validation_at(&self, at: &Opened) -> Result<Option<Delta>, SessionError> {
        let doc = self.format_doc(at.id())?;
        match (Word::of(&*doc), at.any().snapshot::<WordDoc>()) {
            (Some(word), Some(version)) => Ok(Some(word.validation(version)?)),
            _ => Ok(None),
        }
    }

    fn last_check(&self, id: &DocId) -> Result<Option<Arc<ooxml_tools_word::LastCheck>>, SessionError> {
        let doc = self.format_doc(id)?;
        Ok(Word::of(&*doc).map(|word| Arc::clone(&word.last_check)))
    }
}

/// The session's Word format (what it lays its Word documents out with by default, the
/// application's settings), or the default one where it holds none.
fn format_of(session: &Session) -> crate::WordFormat {
    session
        .formats()
        .iter()
        .find_map(|f| f.as_any().downcast_ref::<crate::WordFormat>())
        .cloned()
        .unwrap_or_default()
}

/// Whether `error` is the refusal of an id state given with a document's bytes (they open
/// alone, or with the state saved beside them).
#[must_use]
pub fn refuses_state(error: &SessionError) -> bool {
    matches!(edit_error(error), Some(EditError::IdState(_)))
}

/// Whether `error` says the bytes are a package without a Word document's main part.
#[must_use]
pub fn not_word(error: &SessionError) -> bool {
    matches!(
        edit_error(error),
        Some(EditError::Word(ooxml_word_model::WordError::NoMainPart(_)))
    )
}

/// The edit core's error `error` carries, if it does.
fn edit_error(error: &SessionError) -> Option<&EditError> {
    match error {
        SessionError::Format(error) => error.downcast_ref(),
        _ => None,
    }
}

/// What a loaded state retired, as the session's record takes it: each block id and drawing
/// number with the version that retired it (the host's number) and who made that version.
pub(crate) fn retired_rows(loaded: &LoadedState) -> Vec<(HeldId, u64, Option<String>)> {
    let said = |step: i64| loaded.steps.get(&step).cloned().flatten();
    let version = |step: i64| u64::try_from(step).unwrap_or_default();
    let blocks = loaded
        .gone
        .iter()
        .map(|(v, s)| (HeldId::Block(*v), version(*s), said(*s)));
    let drawings = loaded
        .drawings_gone
        .iter()
        .map(|(n, s)| (HeldId::Drawing(*n), version(*s), said(*s)));
    blocks.chain(drawings).collect()
}
