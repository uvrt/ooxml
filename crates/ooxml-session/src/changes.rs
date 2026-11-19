//! Owns: what one version of a document changed from another, part by part, as bytes
//! ([`Opened::changed_since`]): what a host that keeps a copy of the document beside the
//! session (the bridge's lazily made view, section 13) takes to follow an edit made here, and
//! their names alone ([`Opened::parts_changed_since`]), and the ids given and renamed since
//! ([`Opened::ids_since`]), and what it touched ([`Opened::touched_since`]).
//!
//! Must not: read a part both versions share (decided by identity, `PartVersion::same`), or
//! change either version.

use crate::{Opened, SessionError};
use serde_json::{Map, Value};
use std::sync::Arc;

/// A part's bytes at the later version, or `None` where that version has no such part.
pub type ChangedPart = (String, Option<Arc<[u8]>>);

impl Opened {
    /// The parts whose content differs between `before` and this version of the same
    /// document: each with its bytes here (as its tree writes them), or `None` for a part
    /// `before` has and this version lacks. In this version's part order, removals last.
    ///
    /// # Errors
    /// [`SessionError::Package`] when a part cannot be read; [`SessionError::NotOpen`]
    /// when `before` is a version of another document.
    pub fn changed_since(&self, before: &Opened) -> Result<Vec<ChangedPart>, SessionError> {
        let names = self.parts_changed_since(before)?;
        let package = self.package();
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            let bytes = if package.has_part(&name) { package.read(&name)? } else { None };
            out.push((name, bytes));
        }
        Ok(out)
    }

    /// The names of the parts [`changed_since`](Self::changed_since) gives, without
    /// writing any: what a host that follows the versions by name (its steps' `parts`)
    /// needs, at no cost per byte.
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

    /// The ids this version has given that `before` had not (values of the Word id pool,
    /// in order) and its renames that differ from `before`'s (old id, the id it names
    /// now): what a host's view of the document takes from the session to give no id
    /// twice and follow renamed ids. Empty for a document that is not a Word document.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`] when `before` is a version of another document.
    pub fn ids_since(&self, before: &Opened) -> Result<IdsSince, SessionError> {
        if before.id() != self.id() {
            return Err(SessionError::NotOpen {
                id: before.id().to_string(),
                open: vec![self.id().to_string()],
            });
        }
        let (Some(now), Some(then)) = (self.word(), before.word()) else {
            return Ok(IdsSince::default());
        };
        let (now, then) = (now.ids(), then.ids());
        let given = now.added().difference(then.added()).copied().collect();
        let renamed = now
            .aliases()
            .iter()
            .filter(|(old, new)| then.aliases().get(*old) != Some(*new))
            .map(|(old, new)| (old.clone(), new.clone()))
            .collect();
        Ok(IdsSince { given, renamed })
    }
}

/// What [`Opened::ids_since`] finds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdsSince {
    /// Id values given since, in order.
    pub given: Vec<u32>,
    /// Renames made since: an old id and the id it names now.
    pub renamed: Vec<(String, String)>,
}

impl Opened {
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
fn touched_json(found: &ooxml_word_edit::touched::Touched) -> Value {
    let mut entry = Map::new();
    entry.insert("part".into(), Value::from(found.part.as_str()));
    if let Some(section) = &found.section {
        entry.insert("heading".into(), section.heading.as_deref().map_or(Value::Null, Value::from));
        entry.insert("level".into(), Value::from(section.level));
        entry.insert("blocks".into(), Value::from(section.blocks.to_vec()));
    }
    Value::Object(entry)
}
