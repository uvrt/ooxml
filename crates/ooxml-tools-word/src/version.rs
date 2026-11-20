//! Owns: [`Version`], one version of a Word document as a reading tool is handed it: its
//! name, its package, its number and the document's kept readings, from which the
//! version's [`Docx`] is taken (read once per version; a new version reads only what
//! changed); where the call may ask where its blocks are laid out ([`Placer`]); the
//! version as the session's editor holds it, where it owns the document ([`Owned`]); and
//! [`Others`], the other documents the call names, at the versions pinned for it.
//!
//! Must not: hold documents or sessions (`ooxml-session` does, and makes a `Version` per
//! call).

use crate::placements::Placer;
use ooxml_opc::Package;
use ooxml_tools::ToolError;
use ooxml_word_edit::validate::Problem;
use ooxml_word_edit::WordDoc;
use ooxml_word_model::docx::{Docx, Versions};
use std::sync::Arc;

/// One version of a Word document, for a reading tool.
#[derive(Clone, Copy)]
pub struct Version<'a> {
    /// The document's name, as opened.
    pub name: &'a str,
    /// The package at this version.
    pub package: &'a Package,
    /// The version's number.
    pub number: u64,
    /// The document's kept readings.
    pub versions: &'a Versions,
    /// Where the call may ask where blocks are laid out (`None`: nowhere; a tool that needs
    /// it refuses with the call that works without).
    pub placer: Option<&'a dyn Placer>,
    /// The version as the session's editor holds it, where the session owns the document
    /// (`None`: a package the session only reads; a tool that needs it leaves the call to
    /// the host).
    pub owned: Option<Owned<'a>>,
    /// The other open documents the call names (`ooxml_tools::documents`), each at the
    /// version the session pinned for the call.
    pub others: Others<'a>,
    /// The document at this version as the session holds it, where it holds it as a Word
    /// document (`None`: only its package): its ids and their renames, so that an address
    /// in it resolves as the document's own tools resolve it.
    pub word: Option<&'a ooxml_word_edit::WordDoc>,
}

impl Version<'_> {
    /// This version's reading.
    ///
    /// # Errors
    /// `internal` when the package has no main part or a story part is not XML.
    pub fn docx(&self) -> Result<Arc<Docx>, ToolError> {
        self.versions
            .docx(self.number, self.package)
            .map_err(|e| ToolError::internal(e.to_string(), "word:model"))
    }
}

/// A version of a document the session owns, as its editor holds it: what validation
/// reads, and the problems the document was opened with (the save gate's baseline).
#[derive(Clone, Copy)]
pub struct Owned<'a> {
    /// The version.
    pub doc: &'a WordDoc,
    /// The problems the document had when the session opened it.
    pub baseline: &'a [Problem],
}

/// The other documents a call names, by id, each at the version the session pinned for the
/// call when it dispatched it (versions are immutable: nothing is locked).
#[derive(Clone, Copy, Default)]
pub struct Others<'a> {
    versions: &'a [(&'a str, Version<'a>)],
}

impl<'a> Others<'a> {
    /// The documents `versions` holds, by id.
    #[must_use]
    pub fn new(versions: &'a [(&'a str, Version<'a>)]) -> Self {
        Self { versions }
    }

    /// The document `id` at its pinned version (`None`: the call names no such open
    /// document; the session refused a call naming one not open).
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Version<'a>> {
        self.versions.iter().find(|(named, _)| *named == id).map(|(_, v)| v)
    }
}
