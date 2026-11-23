//! Owns: a new document's bytes made from a template's ([`Session::new_from_template`],
//! docx-agent's `new_document` with `template_blob`: `Document.new(template=...,
//! keep_content=False)`): the template opened from the session's own copy of its bytes,
//! made a new document by the edit core ([`NewFromTemplate`]) and written as a `.docx`.
//!
//! Must not: open the new document in the session (the host opens the bytes, as it opens
//! a blank one), or change the bytes it is given.

use crate::context::StoreClock;
use crate::document::Budget;
use crate::{Session, SessionError};
use ooxml_opc::Package;
use ooxml_store::Caller;
use ooxml_word_edit::{serialise, EditError, Editor, FileKind, NewFromTemplate};
use std::sync::Arc;

impl Session {
    /// A new document's bytes from `template` (a `.dotx`, `.dotm`, `.docx` or `.docm`'s),
    /// as `made` says: its content left out, its properties started again, a `.docx`.
    ///
    /// # Errors
    /// [`SessionError::Package`] for bytes that are not a package within the limits;
    /// [`SessionError::Word`] for a package that is not a Word document or template.
    pub fn new_from_template(&self, template: Arc<[u8]>, made: &NewFromTemplate) -> Result<Vec<u8>, SessionError> {
        let package = Package::open(template, &self.context.limits)?;
        if FileKind::of(&package)?.is_none() {
            return Err(EditError::Refused("not a Word document or template".to_owned()).into());
        }
        let clock = Arc::new(StoreClock(Arc::clone(&self.context.clock)));
        let budget = Arc::new(Budget(Arc::clone(&self.context.budget)));
        let editor = Editor::open(package, self.config.store, clock, budget)?;
        editor.run(&Caller::anonymous(), made)?;
        let head = editor.store().snapshot();
        Ok(serialise(head.doc().package(), FileKind::Docx, None)?)
    }
}
