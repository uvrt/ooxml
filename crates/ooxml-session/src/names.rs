//! Owns: the documents a call names, found as its host names them ([`DocNames`]: each
//! name the host gives a document -- `d1` in the host's own session -- and the session's
//! id of that document), the call's own document and the others it names, each at the
//! version pinned for the call (else its head).
//!
//! Must not: put the session's id where the host's name was given: a reply names a
//! document as the call named it (its undo and way-out calls, its summary, its `doc`), so
//! a host whose names are not the session's ids (many host sessions over one session)
//! gets its own names back.

use crate::dispatch::Pinned;
use crate::Session;
use crate::{DocId, Opened, SessionError};
use ooxml_tools::{ToolError, ToolSpec};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The host's names for the documents a call names, each the session's id of that
/// document. Empty: the call names documents by the session's ids.
#[derive(Clone, Debug, Default)]
pub struct DocNames(BTreeMap<String, DocId>);

impl DocNames {
    /// No names: documents are named by the session's ids.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `name` names the document the session knows as `id`.
    pub fn insert(&mut self, name: impl Into<String>, id: DocId) {
        self.0.insert(name.into(), id);
    }

    /// The document `given` names: where the host names documents, only by its names;
    /// else by the session's id.
    #[must_use]
    pub fn resolve(&self, given: &str) -> Option<DocId> {
        if self.0.is_empty() {
            DocId::parse(given)
        } else {
            self.0.get(given).cloned()
        }
    }

    /// The names the call gives the documents `ids` (by the session's ids where the host
    /// names none; a document it does not name is left out).
    pub(crate) fn of<'a>(&self, ids: impl Iterator<Item = &'a DocId>) -> Vec<String> {
        if self.0.is_empty() {
            return ids.map(ToString::to_string).collect();
        }
        let ids: Vec<&DocId> = ids.collect();
        self.0
            .iter()
            .filter(|(_, id)| ids.contains(id))
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// `given` not open, with the names the call may give instead (the host's, where it
    /// names documents).
    fn not_open(&self, session: &Session, given: &str) -> SessionError {
        if self.0.is_empty() {
            return session.not_open(given);
        }
        SessionError::NotOpen {
            id: given.to_owned(),
            open: self.0.keys().cloned().collect(),
        }
    }
}

impl Session {
    /// The document a call names (its first document argument) and the name it gives it:
    /// at the version `pinned` holds for it, else at its head (then pinned there).
    pub(crate) fn document_of(
        &self,
        spec: &ToolSpec,
        arguments: &Map<String, Value>,
        names: &DocNames,
        pinned: Option<&mut Pinned>,
    ) -> Result<(String, Opened), ToolError> {
        let named = ooxml_tools::documents::named(&spec.documents, arguments);
        let Some(given) = named.first() else {
            return Err(ToolError::internal(
                format!("{} names no document; the session serves only tools that do", spec.name),
                "session:documents",
            ));
        };
        let id = names
            .resolve(given)
            .ok_or_else(|| names.not_open(self, given).to_tool_error())?;
        let found = self.pinned(&id, pinned).map_err(|e| e.to_tool_error())?;
        Ok((given.clone(), found))
    }

    /// The other documents a call names (after its own, each once), by the names it gives
    /// them, each at the version `pinned` holds for it, else at its head (then pinned
    /// there): a document not open is refused at the field that names it.
    pub(crate) fn others_of(
        &self,
        spec: &ToolSpec,
        arguments: &Map<String, Value>,
        own: &DocId,
        names: &DocNames,
        mut pinned: Option<&mut Pinned>,
    ) -> Result<Vec<(String, Opened)>, ToolError> {
        let mut out: Vec<(String, Opened)> = Vec::new();
        for (field, given) in ooxml_tools::documents::named_at(&spec.documents, arguments)
            .into_iter()
            .skip(1)
        {
            let refuse = |e: SessionError| e.to_tool_error().at(field);
            let id = names
                .resolve(&given)
                .ok_or_else(|| refuse(names.not_open(self, &given)))?;
            if id == *own || out.iter().any(|(seen, _)| *seen == given) {
                continue;
            }
            let found = self.pinned(&id, pinned.as_deref_mut()).map_err(refuse)?;
            out.push((given, found));
        }
        Ok(out)
    }

    /// The document `id` at the version `pinned` holds for it, else at its head (then
    /// pinned there).
    fn pinned(&self, id: &DocId, pinned: Option<&mut Pinned>) -> Result<Opened, SessionError> {
        let Some(pinned) = pinned else {
            return self.snapshot(id);
        };
        if let Some(found) = pinned.get(id) {
            return Ok(found.clone());
        }
        let found = self.snapshot(id)?;
        pinned.insert(id.clone(), found.clone());
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_names_the_sessions_ids_are_the_names() {
        let names = DocNames::new();
        assert_eq!(names.resolve("d3"), Some(DocId::nth(3)));
        assert_eq!(names.resolve("report"), None);
    }

    #[test]
    fn with_names_only_the_hosts_names_are() {
        let mut names = DocNames::new();
        names.insert("d1", DocId::nth(3));
        assert_eq!(names.resolve("d1"), Some(DocId::nth(3)));
        // (the session's own id is another host's name, not this one's)
        assert_eq!(names.resolve("d3"), None);
    }
}
