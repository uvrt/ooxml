//! Owns: what one version of a document changed from another: the parts whose content
//! differs, by name ([`Opened::parts_changed_since`]).
//!
//! Must not: read a part both versions share (decided by identity, `PartVersion::same`), or
//! change either version.

use crate::{Opened, SessionError};

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
        Ok(changed(before.package(), self.package()))
    }
}

/// The parts whose bytes differ between `then` and `now` (the session's `changed_parts`): changed or added ones in `now`'s
/// order, then the removed ones.
pub fn changed(then: &ooxml_opc::Package, now: &ooxml_opc::Package) -> Vec<String> {
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
    out
}
