//! Owns: a new document's bytes made from a template's ([`FromTemplate::new_from_template`],
//! docx-agent's `new_document` with `template_blob`: `Document.new(template=...,
//! keep_content=False)`): the template opened from the session's own copy of its bytes,
//! made a new document by the edit core ([`NewFromTemplate`]) and written as a `.docx`.
//!
//! Must not: open the new document in the session (the host opens the bytes, as it opens
//! a blank one), or change the bytes it is given.

use crate::word::Budget;
use ooxml_opc::Package;
use ooxml_session::{Session, SessionError, StoreClock};
use ooxml_store::Caller;
use ooxml_word_edit::{serialise, EditError, Editor, FileKind, NewFromTemplate};
use std::sync::Arc;

/// A new Word document made from a template's bytes.
pub trait FromTemplate {
    /// A new document's bytes from `template` (a `.dotx`, `.dotm`, `.docx` or `.docm`'s),
    /// as `made` says: its content left out, its properties started again, a `.docx`.
    ///
    /// # Errors
    /// [`SessionError::Package`] for bytes that are not a package within the limits;
    /// [`SessionError::Word`] for a package that is not a Word document or template.
    fn new_from_template(&self, template: Arc<[u8]>, made: &NewFromTemplate) -> Result<Vec<u8>, SessionError>;
}

impl FromTemplate for Session {
    fn new_from_template(&self, template: Arc<[u8]>, made: &NewFromTemplate) -> Result<Vec<u8>, SessionError> {
        let package = Package::open(template, &self.context().limits)?;
        if FileKind::of(&package).map_err(SessionError::format)?.is_none() {
            return Err(SessionError::format(EditError::Refused(
                "not a Word document or template".to_owned(),
            )));
        }
        let clock = Arc::new(StoreClock(Arc::clone(&self.context().clock)));
        let budget = Arc::new(Budget(Arc::clone(&self.context().budget)));
        let editor = Editor::open(package, self.config().store, clock, budget).map_err(SessionError::format)?;
        editor.run(&Caller::anonymous(), made).map_err(SessionError::format)?;
        let head = editor.store().snapshot();
        serialise(head.doc().package(), FileKind::Docx, None).map_err(SessionError::format)
    }
}
