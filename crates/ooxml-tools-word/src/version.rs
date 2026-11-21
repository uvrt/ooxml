//! Owns: [`Version`], one version of a Word document as a reading tool is handed it: its
//! name, its package, its number and the document's kept readings, from which the
//! version's [`Docx`] is taken (read once per version; a new version reads only what
//! changed); where the call may ask where its blocks are laid out ([`Placer`]); the
//! version as the session's editor holds it, where it owns the document ([`Owned`]); and
//! [`Others`], the other documents the call names, at the versions pinned for it.
//!
//! Must not: hold documents or sessions (`ooxml-session` does, and makes a `Version` per
//! call).

use crate::check::LastCheck;
use crate::pages::WordLayout;
use crate::placements::Placer;
use ooxml_opc::Package;
use ooxml_tools::{Limits, ToolError};
use ooxml_word_edit::validate::Problem;
use ooxml_word_edit::WordDoc;
use ooxml_word_model::docx::{Docx, Versions};
use std::sync::Arc;

/// One version of a Word document, for a reading tool. Made with [`Version::new`] and the
/// `with_*` setters, never as a struct literal outside this crate (`non_exhaustive`), so a
/// new kind of context adds a setter, not a field to every caller's literal.
#[derive(Clone, Copy)]
#[non_exhaustive]
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
    pub word: Option<&'a WordDoc>,
    /// The host's limits on the call's reply (a page's characters, a listing's items).
    pub limits: Limits,
    /// The pages and lines of this version's layout, where the host has laid it out (the
    /// statistics `save_document` writes into `app.xml`; `None`: not laid out, left as they
    /// are).
    pub counts: Option<LayoutCounts>,
}

/// The pages and lines of a version's layout, as the host that laid it out counts them
/// (docx-agent's `layout_statistics`: the pages known, the body's lines on them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutCounts {
    /// The pages.
    pub pages: u64,
    /// The body's lines.
    pub lines: u64,
}

impl<'a> Version<'a> {
    /// What every version is: its document's name, its package, its number and the
    /// document's kept readings. The rest -- a placer, the session's own version, other
    /// documents, the document as a Word document -- starts empty, the limits at their
    /// defaults, and is given with the `with_*` setters.
    #[must_use]
    pub fn new(name: &'a str, package: &'a Package, number: u64, versions: &'a Versions) -> Self {
        Self {
            name,
            package,
            number,
            versions,
            placer: None,
            owned: None,
            others: Others::default(),
            word: None,
            limits: Limits::DEFAULT,
            counts: None,
        }
    }

    /// Where the call may ask where blocks are laid out (`None`: nowhere).
    #[must_use]
    pub fn with_placer(mut self, placer: Option<&'a dyn Placer>) -> Self {
        self.placer = placer;
        self
    }

    /// The version as the session's editor holds it, where the session owns the document.
    #[must_use]
    pub fn with_owned(mut self, owned: Option<Owned<'a>>) -> Self {
        self.owned = owned;
        self
    }

    /// The other open documents the call names, at their pinned versions.
    #[must_use]
    pub fn with_others(mut self, others: Others<'a>) -> Self {
        self.others = others;
        self
    }

    /// The document at this version as the session holds it as a Word document.
    #[must_use]
    pub fn with_word(mut self, word: Option<&'a WordDoc>) -> Self {
        self.word = word;
        self
    }

    /// The host's limits on the call's reply.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// The pages and lines of this version's layout, where the host has laid it out.
    #[must_use]
    pub fn with_counts(mut self, counts: Option<LayoutCounts>) -> Self {
        self.counts = counts;
        self
    }
}

impl Version<'_> {
    /// `address` as it names a block now: through the renames the session's edits made
    /// (`p@body/1` once stamped `p:X`; a row's or cell's table), as docx-agent's view reads
    /// an address; as given on a version the session only reads.
    #[must_use]
    pub fn follow(&self, address: &str) -> String {
        self.owned
            .map_or_else(|| address.to_owned(), |owned| owned.doc.ids().follow_address(address))
    }

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
/// reads, the problems the document was opened with (the save gate's baseline), what the
/// document's last check saw, the host's layout of its versions where the call was
/// handed one, and the document as opened (what the records this session wrote are told
/// from).
#[derive(Clone, Copy)]
pub struct Owned<'a> {
    /// The version.
    pub doc: &'a WordDoc,
    /// The document as the session opened it: what a pending change of the session's takes
    /// out of the file is read against, and what changed since (`save_document`'s `app.xml`
    /// refreshed only when the main part did).
    pub opened: &'a WordDoc,
    /// The problems the document had when the session opened it.
    pub baseline: &'a [Problem],
    /// What the document's last check with `reflow` saw.
    pub last_check: &'a LastCheck,
    /// The host's layout of the document's versions (`None`: the call was handed none).
    pub layout: Option<&'a dyn WordLayout>,
    /// How long laying a version out may take in the call (`None`: as long as the call).
    pub layout_limit: Option<crate::LayoutLimit<'a>>,
    /// What the call draws pages with, where the host hands a raster (`render`).
    pub drawing: Option<crate::Drawing<'a>>,
}

/// A table of another open document the host holds itself (a workbook's `t:stock`), as a
/// `from` names it: bare where one open document alone takes its prefix, else with its
/// document (`d2/t:stock`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostTable {
    /// The table as a `from` names it (`t:stock`, `d2/t:stock`).
    pub name: String,
    /// The host's name of its document (`d2`).
    pub doc: String,
    /// Its document's format (`xlsx`).
    pub kind: String,
}

impl HostTable {
    /// Whether `given` names this table: as a `from` names it, or with its document
    /// (`d2/t:stock` for a bare `t:stock`).
    #[must_use]
    pub fn named_by(&self, given: &str) -> bool {
        let prefix = format!("{}/", self.doc);
        let address = self.name.strip_prefix(&prefix).unwrap_or(&self.name);
        given == self.name || given.strip_prefix(&prefix) == Some(address)
    }
}

/// The tables of the open documents the host holds itself, asked only where a call needs
/// them (a refusal that names one): reading another document's tables may wait for it.
pub trait HostTables: Send + Sync {
    /// The tables there are now (none where the host cannot read them).
    fn tables(&self) -> Vec<HostTable>;
}

/// The other documents a call names, by id, each at the version the session pinned for the
/// call when it dispatched it (versions are immutable: nothing is locked), and the tables
/// of the open documents the host holds itself.
#[derive(Clone, Copy, Default)]
pub struct Others<'a> {
    versions: &'a [(&'a str, Version<'a>)],
    tables: Option<&'a dyn HostTables>,
}

impl<'a> Others<'a> {
    /// The documents `versions` holds, by id.
    #[must_use]
    pub fn new(versions: &'a [(&'a str, Version<'a>)]) -> Self {
        Self { versions, tables: None }
    }

    /// The tables of the open documents the host holds itself (none unless given).
    #[must_use]
    pub fn with_tables(mut self, tables: Option<&'a dyn HostTables>) -> Self {
        self.tables = tables;
        self
    }

    /// The host's table `given` names (`t:stock`, `d2/t:stock`), where it holds one.
    #[must_use]
    pub fn host_table(&self, given: &str) -> Option<HostTable> {
        self.tables?.tables().into_iter().find(|t| t.named_by(given))
    }

    /// The document `id` at its pinned version (`None`: the call names no such open
    /// document; the session refused a call naming one not open).
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Version<'a>> {
        self.versions.iter().find(|(named, _)| *named == id).map(|(_, v)| v)
    }
}

#[cfg(test)]
mod tests {
    use super::HostTable;

    fn table(name: &str) -> HostTable {
        HostTable {
            name: name.to_owned(),
            doc: "d2".to_owned(),
            kind: "xlsx".to_owned(),
        }
    }

    #[test]
    fn a_host_table_is_named_as_from_takes_it_or_with_its_document() {
        let bare = table("t:stock");
        assert!(bare.named_by("t:stock") && bare.named_by("d2/t:stock"));
        assert!(!bare.named_by("d3/t:stock") && !bare.named_by("t:stocks"));
        // a name shared by several documents is only named with its own
        let shared = table("d2/t:stock");
        assert!(shared.named_by("d2/t:stock") && !shared.named_by("t:stock"));
    }
}
