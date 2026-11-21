//! Owns: the session's side of a Word document's rows written (`ooxml_tools_word::
//! RowsWritten`, round 58, R126): read for `check`'s `row_text_format`
//! ([`Session::rows_written`]) and added to where the host answered a table call itself
//! ([`Session::note_rows_written`]), so that one record holds both writers' rows. The
//! record is the session's, beside the document's versions: it is never pruned, and a
//! version that does not hold a row named in it simply does not find that row.
//!
//! Must not: read or change a version.

use crate::{DocId, Session, SessionError};
use std::collections::BTreeSet;

impl Session {
    /// The rows of the Word document this session's `word_edit_table` calls added or wrote
    /// into, by `w14:paraId` (check's `row_text_format` reads them); none for another kind.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn rows_written(&self, id: &DocId) -> Result<BTreeSet<String>, SessionError> {
        let document = self.document(id)?;
        Ok(document.word().map(|w| w.rows_written.ids()).unwrap_or_default())
    }

    /// Keep `rows` (`w14:paraId`s) as written by this session: the rows a table call the
    /// host answered itself added or wrote into, in the same record as the session's own.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn note_rows_written(&self, id: &DocId, rows: Vec<String>) -> Result<(), SessionError> {
        let document = self.document(id)?;
        if let Some(word) = document.word() {
            word.rows_written.note(rows);
        }
        Ok(())
    }
}
