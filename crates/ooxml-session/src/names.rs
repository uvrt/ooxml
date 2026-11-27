//! Owns: the documents a call names, by the session's ids: the call's own document and the
//! others it names, each at the version pinned for the call (else its head).
//!
//! Must not: name a document otherwise than as the call named it (its undo and way-out
//! calls, its summary, its `doc`).

use crate::dispatch::Pinned;
use crate::Session;
use crate::{DocId, Opened, SessionError};
use ooxml_tools::{ToolError, ToolSpec};
use serde_json::{Map, Value};

impl Session {
    /// The document a call names (its first document argument) and the name it gives it:
    /// at the version `pinned` holds for it, else at its head (then pinned there).
    pub(crate) fn document_of(
        &self,
        spec: &ToolSpec,
        arguments: &Map<String, Value>,
        pinned: Option<&mut Pinned>,
    ) -> Result<(String, Opened), ToolError> {
        let named = ooxml_tools::documents::named(&spec.documents, arguments);
        let Some(given) = named.first() else {
            return Err(ToolError::internal(
                format!("{} names no document; the session serves only tools that do", spec.name),
                "session:documents",
            ));
        };
        let id = DocId::parse(given).ok_or_else(|| self.not_open(given).to_tool_error())?;
        let found = self.pinned(&id, pinned).map_err(|e| e.to_tool_error())?;
        Ok((given.clone(), found))
    }

    /// The other documents a call names (after its own, each once), each at the version
    /// `pinned` holds for it, else at its head (then pinned there): a document not open is
    /// refused at the field that names it.
    pub(crate) fn others_of(
        &self,
        spec: &ToolSpec,
        arguments: &Map<String, Value>,
        own: &DocId,
        mut pinned: Option<&mut Pinned>,
    ) -> Result<Vec<(String, Opened)>, ToolError> {
        let mut out: Vec<(String, Opened)> = Vec::new();
        for (field, given) in ooxml_tools::documents::named_at(&spec.documents, arguments)
            .into_iter()
            .skip(1)
        {
            let refuse = |e: SessionError| e.to_tool_error().at(field);
            let id = DocId::parse(&given).ok_or_else(|| refuse(self.not_open(&given)))?;
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
