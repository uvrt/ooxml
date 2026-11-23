//! Owns: what one version of a document changed from another: the parts whose content
//! differs, by name ([`Opened::parts_changed_since`]), and what it touched
//! ([`Opened::touched_since`]).
//!
//! Must not: read a part both versions share (decided by identity, `PartVersion::same`), or
//! change either version.

use crate::{Opened, SessionError};
use serde_json::{Map, Value};

impl Opened {
    /// The names of the parts whose content differs between `before` and this version of
    /// the same document, none written: in this version's part order, those `before` has and
    /// this version lacks last.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`] when `before` is a version of another document.
    pub fn parts_changed_since(&self, before: &Opened) -> Result<Vec<String>, SessionError> {
        if before.id() != self.id() {
            return Err(SessionError::NotOpen {
                id: before.id().to_string(),
                open: vec![self.id().to_string()],
            });
        }
        let (now, then) = (self.package(), before.package());
        let mut out = Vec::new();
        for name in now.part_names() {
            let same = match (now.part(&name), then.part(&name)) {
                (Some(here), Some(there)) => here.same(there),
                _ => false,
            };
            if !same {
                out.push(name);
            }
        }
        out.extend(then.part_names().into_iter().filter(|name| !now.has_part(name)));
        Ok(out)
    }

    /// What changed from `before` to this version, as docx-agent's `Result.touched` says it:
    /// each part whose content differs, and for the body the heading sections holding the
    /// blocks that changed (`ooxml_word_edit::touched`, found from the change, no other block
    /// read). Empty for a document that is not a Word document.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`] when `before` is a version of another document; an edit
    /// error when a changed part cannot be read.
    pub fn touched_since(&self, before: &Opened) -> Result<Vec<Value>, SessionError> {
        if before.id() != self.id() {
            return Err(SessionError::NotOpen {
                id: before.id().to_string(),
                open: vec![self.id().to_string()],
            });
        }
        let (Some(now), Some(then)) = (self.word(), before.word()) else {
            return Ok(Vec::new());
        };
        let found = ooxml_word_edit::touched::touched(then, now)?;
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
