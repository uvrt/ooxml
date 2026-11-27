//! Owns: a version of a Word document as Word's code reads it ([`WordVersion`]): its Word
//! document, the [`Version`] a reading tool is handed, and what it touched since another
//! version (docx-agent's `Result.touched`, [`touched_json`]).
//!
//! Must not: change the version.

use ooxml_session::{Opened, SessionError};
use ooxml_tools_word::Version;
use ooxml_word_edit::WordDoc;
use ooxml_word_model::docx::Versions;
use ooxml_word_model::WordPackage;
use serde_json::{Map, Value};
use std::cell::OnceCell;
use std::sync::OnceLock;

/// A version of a document as Word's code reads it.
pub trait WordVersion {
    /// The Word document at this version, when the session owns it as one.
    fn word(&self) -> Option<&WordDoc>;

    /// This version as a Word reading tool is handed it: a document the session holds as a
    /// Word document as its editor reads it, any other package read into `read` (its blocks
    /// stamped as an open stamps them, `ooxml_word_model::WordPackage::open`).
    fn version_of<'a>(&'a self, read: &'a OnceCell<WordPackage>) -> Version<'a>;

    /// What changed from `before` to this version, as docx-agent's `Result.touched` says it:
    /// each part whose content differs, and for the body the heading sections holding the
    /// blocks that changed (`ooxml_word_edit::touched`, found from the change, no other block
    /// read). Empty for a document that is not a Word document.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`] when `before` is a version of another document; an edit
    /// error when a changed part cannot be read.
    fn touched_since(&self, before: &Opened) -> Result<Vec<Value>, SessionError>;
}

impl WordVersion for Opened {
    fn word(&self) -> Option<&WordDoc> {
        self.any().snapshot::<WordDoc>().map(|v| v.doc())
    }

    fn version_of<'a>(&'a self, read: &'a OnceCell<WordPackage>) -> Version<'a> {
        // (a document Word does not keep readings of: none kept)
        static NONE: OnceLock<Versions> = OnceLock::new();
        let versions = self
            .readings()
            .downcast_ref::<Versions>()
            .unwrap_or_else(|| NONE.get_or_init(Versions::new));
        let package = match self.word() {
            Some(word) => word.package(),
            None => read.get_or_init(|| WordPackage::open(self.package().clone()).0),
        };
        Version::new(self.name(), package, self.version(), versions).with_word(self.word())
    }

    fn touched_since(&self, before: &Opened) -> Result<Vec<Value>, SessionError> {
        if before.id() != self.id() {
            return Err(SessionError::NotOpen {
                id: before.id().to_string(),
                open: vec![self.id().to_string()],
            });
        }
        let (Some(now), Some(then)) = (self.word(), before.word()) else {
            return Ok(Vec::new());
        };
        let found = ooxml_word_edit::touched::touched(then, now).map_err(SessionError::format)?;
        Ok(found.iter().map(touched_json).collect())
    }
}

/// One `touched` entry as docx-agent's `tools/touched.py` writes it.
pub(crate) fn touched_json(found: &ooxml_word_edit::touched::Touched) -> Value {
    let mut entry = Map::new();
    entry.insert("part".into(), Value::from(found.part.as_str()));
    if let Some(section) = &found.section {
        entry.insert(
            "heading".into(),
            section.heading.as_deref().map_or(Value::Null, Value::from),
        );
        entry.insert("level".into(), Value::from(section.level));
        entry.insert("blocks".into(), Value::from(section.blocks.to_vec()));
    }
    Value::Object(entry)
}
