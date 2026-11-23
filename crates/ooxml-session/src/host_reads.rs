//! Owns: what a host's tool layer reads of a Word version beside the tools (lane H7a, the
//! format hooks of docx-agent's `tools/format.py` without its Python view): the version's
//! validation problems ([`Opened::problems`]), what `open_document` says of it
//! ([`Opened::summary`]) and its id state ([`Opened::id_state`], docx-agent's
//! `Document.id_state`); and, for a source value that names the document's own things
//! (lane H7d, docx-agent's `own_source` and `source_hint`), what kind of block an id names
//! ([`Opened::block_kind`]) and whether a drawing id is there ([`Opened::has_drawing`]).
//!
//! Must not: change the document, or number versions (the host's [`IdStateOf`] says how it
//! numbers them).

use crate::{CancelToken, Opened, SessionError};
use ooxml_tools::ToolError;
use ooxml_word_edit::validate::Problem;
use ooxml_word_edit::FileKind;
use ooxml_word_model::address::direct;
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::drawings::all_drawings;
use ooxml_word_model::ids::Entry;
use ooxml_word_model::ids::{export_state, IdPool, Ledger, Versions};
use ooxml_word_model::parts::parts;
use ooxml_xml::Tree;
use serde_json::Value;
use std::collections::HashMap;

/// What a host says of a Word document beside its version for its id state: the host's
/// numbers of the version and of the highest state given, and the retired ids as it numbers
/// them (`ooxml-edit` numbers states; the session numbers versions) -- the session's own
/// record ([`crate::Session::retired`]) read through the host's numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdStateOf {
    /// The version, as the host numbers it.
    pub version: i64,
    /// The highest state number the host gave.
    pub last: i64,
    /// Each retired block id value, with the host's number of the version that removed it.
    pub gone: Vec<(u32, i64)>,
    /// Each retired drawing number, with the host's number of the version that removed it.
    pub drawings_gone: Vec<(u64, i64)>,
    /// Who made each change made before the document was saved and opened again, by the
    /// host's number, in the words its state kept (`None`: not known).
    pub earlier: Vec<(i64, Option<String>)>,
    /// Who made each other change that retired an id, by the host's number, in words.
    pub said: Vec<(i64, String)>,
}

impl Opened {
    /// The version's validation problems (what the save gate compares with those the
    /// document opened with); `None` for a document that is not a Word document.
    ///
    /// # Errors
    /// The edit core's when the version cannot be read.
    pub fn problems(&self) -> Result<Option<Vec<Problem>>, SessionError> {
        let Some(word) = self.word() else {
            return Ok(None);
        };
        Ok(Some(ooxml_word_edit::validate::problems(word)?))
    }

    /// The Word file kind the version's package is (`docx`, `docm`, `dotx`, `dotm`: its main
    /// part's content type); `None` for a document that is not a Word document, or a main part
    /// of no Word kind.
    ///
    /// # Errors
    /// The package's when its content types cannot be read.
    pub fn file_kind(&self) -> Result<Option<&'static str>, SessionError> {
        let Some(word) = self.word() else {
            return Ok(None);
        };
        Ok(FileKind::of(word.package())?.map(FileKind::name))
    }

    /// What `open_document` says of the version ([`ooxml_tools_word::summary`]); `None` for
    /// a document that is not a Word document.
    ///
    /// # Errors
    /// `internal` when a part it reads is not XML or the package has no main part.
    pub fn summary(&self) -> Result<Option<Value>, ToolError> {
        let Some(word) = self.word() else {
            return Ok(None);
        };
        let docx = word
            .docx()
            .map_err(|e| ToolError::internal(e.to_string(), "summary:docx"))?;
        ooxml_tools_word::summary(word.package(), &docx, &CancelToken::new()).map(Some)
    }

    /// The version's id state (docx-agent's `Document.id_state`: the string a host keeps
    /// beside its bytes to open them again with), from the ids the session holds and
    /// `host`'s numbers; `None` for a document that is not a Word document. Equal to
    /// docx-agent's as a record (the deflate bytes may differ; either reads the other's).
    ///
    /// # Errors
    /// When a story part is not XML.
    pub fn id_state(&self, host: &IdStateOf) -> Result<Option<String>, SessionError> {
        let Some(word) = self.word() else {
            return Ok(None);
        };
        let package = word.package();
        let mut trees: Vec<Tree> = Vec::new();
        for part in parts(package)? {
            if let Some(tree) = word.tree(&part)? {
                trees.push(tree);
            }
        }
        let roots: Vec<&ooxml_xml::Element> = trees.iter().map(|t| t.root().as_ref()).collect();
        let ids = word.ids();
        let mut pool = IdPool::of(ids.values().collect());
        for (seed, k) in ids.cursors() {
            pool.set_cursor(seed, *k);
        }
        let ledger = Ledger {
            pool: Some(pool),
            gone: host.gone.iter().copied().collect(),
            drawings_gone: host.drawings_gone.iter().copied().collect(),
            highest_drawing: ids.highest_drawing(),
            replaced: ids.replaced().iter().map(|(v, id)| (*v, id.clone())).collect(),
            earlier: host.earlier.iter().cloned().collect(),
            aliases: ids.aliases().clone(),
            ..Ledger::default()
        };
        let said: HashMap<i64, &str> = host.said.iter().map(|(v, w)| (*v, w.as_str())).collect();
        let versions = Versions {
            version: host.version,
            last: host.last,
        };
        let describe = |version: i64| said.get(&version).map(|w| (*w).to_owned());
        Ok(Some(export_state(&ledger, &roots, &versions, &describe)))
    }

    /// What kind of block `id` names in the version (docx-agent's `_resolve_block`):
    /// `"paragraph"`, `"table"`, or `"content control"` for a block-level one (`cc:`,
    /// `cc@`); a renamed id is followed while what it names now is there. `None` when it
    /// names none of the document's blocks, or the document is not a Word document.
    ///
    /// # Errors
    /// When a story part is not XML.
    pub fn block_kind(&self, id: &str) -> Result<Option<&'static str>, SessionError> {
        let Some(word) = self.word() else {
            return Ok(None);
        };
        let docx = word.docx()?;
        if id.starts_with("cc:") || id.starts_with("cc@") {
            let block = controls(&docx)
                .into_iter()
                .find(|c| c.id == id)
                .is_some_and(|c| docx.part(&c.part).is_some_and(|part| c.level(part) == "block"));
            return Ok(block.then_some("content control"));
        }
        let followed = word.ids().follow(id);
        let found = direct(&docx, &followed).or_else(|| direct(&docx, id));
        Ok(found.map(|(_, entry)| match entry {
            Entry::Paragraph(_) => "paragraph",
            Entry::Table(_) => "table",
        }))
    }

    /// Whether the version has the drawing `id` (`d:3`, `d:3#1`; docx-agent's `drawing`:
    /// every drawing with a `docPr` id, a picture under its picture id); `false` for a
    /// document that is not a Word document.
    ///
    /// # Errors
    /// When a story part is not XML.
    pub fn has_drawing(&self, id: &str) -> Result<bool, SessionError> {
        let Some(word) = self.word() else {
            return Ok(false);
        };
        let docx = word.docx()?;
        Ok(all_drawings(&docx).iter().any(|d| d.id == id))
    }
}
