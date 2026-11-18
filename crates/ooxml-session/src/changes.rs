//! Owns: what one version of a document changed from another, part by part, as bytes
//! ([`Opened::changed_since`]): what a host that keeps a copy of the document beside the
//! session (the bridge's working copy, section 13) takes to follow an edit made here.
//!
//! Must not: read a part both versions share (decided by identity, `PartVersion::same`), or
//! change either version.

use crate::{Opened, SessionError};
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
                out.push((name.clone(), now.read(&name)?));
            }
        }
        for name in then.part_names() {
            if !now.has_part(&name) {
                out.push((name, None));
            }
        }
        Ok(out)
    }
}
