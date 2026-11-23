//! Owns: a commit of parts given as bytes ([`crate::Session::update`]): a version's touched
//! parts, parsed here and committed as one transaction and one history
//! step ([`PartUpdate`], a store [`Command`]). A part parsed again shares with the version
//! before every subtree that did not change (step 4: the two trees diffed structurally and
//! the differences applied to the old one by path copying), so what is kept per subtree --
//! block ids, a reading's facts, a layout's measures -- is found again by pointer, as after
//! an edit made here.
//!
//! The update reads the whole document (section 13, step 5): it commits only on the version
//! it was prepared on. Into a Word document it also adds to the id ledger every id value the
//! parts hold that the ledger lacks, so an id the parts brought is never given again.
//!
//! Must not: interpret a part's content beyond parsing its XML and reading its ids, or run
//! anything with effects outside its transaction (a conflict prepares it again).

use crate::model::{PackageDoc, PartChange, PartContent};
use crate::SessionError;
use ooxml_base::{Cancel, Limits};
use ooxml_opc::name::normalize;
use ooxml_opc::Package;
use ooxml_store::{Command, DocModel, Error, Txn};
use ooxml_word_edit::{Change, IdsChange, WordDoc};
use ooxml_xml::Tree;
use std::sync::Arc;

/// A document model parts given as bytes are committed into.
pub(crate) trait Bridged: DocModel {
    /// The version's package.
    fn package(&self) -> &Package;
    /// The change that puts `tree` as `part`.
    fn put_tree(part: String, tree: Tree) -> Self::Change;
    /// The change that puts `bytes` as `part`.
    fn put_bytes(part: String, bytes: Arc<[u8]>) -> Self::Change;
    /// The change that removes `part`.
    fn remove(part: String) -> Self::Change;
    /// What the put trees add to the version's ledger, if anything.
    fn ledger(&self, trees: &[&Tree]) -> Option<Self::Change>;
}

impl Bridged for PackageDoc {
    fn package(&self) -> &Package {
        PackageDoc::package(self)
    }

    fn put_tree(part: String, tree: Tree) -> PartChange {
        PartChange::Put {
            part,
            content: PartContent::Tree(tree),
        }
    }

    fn put_bytes(part: String, bytes: Arc<[u8]>) -> PartChange {
        PartChange::Put {
            part,
            content: PartContent::Bytes(bytes),
        }
    }

    fn remove(part: String) -> PartChange {
        PartChange::Remove { part }
    }

    fn ledger(&self, _trees: &[&Tree]) -> Option<PartChange> {
        None
    }
}

impl Bridged for WordDoc {
    fn package(&self) -> &Package {
        WordDoc::package(self)
    }

    fn put_tree(part: String, tree: Tree) -> Change {
        Change::Put {
            part: Arc::from(part),
            tree,
        }
    }

    fn put_bytes(part: String, bytes: Arc<[u8]>) -> Change {
        Change::Bytes {
            part: Arc::from(part),
            bytes,
        }
    }

    fn remove(part: String) -> Change {
        Change::Remove { part: Arc::from(part) }
    }

    fn ledger(&self, trees: &[&Tree]) -> Option<Change> {
        let roots: Vec<&ooxml_xml::Element> = trees.iter().map(|t| &**t.root()).collect();
        let mut values: Vec<u32> = ooxml_word_model::ids::used_long_hex(&roots)
            .into_iter()
            .filter(|v| !self.ids().contains(*v))
            .collect();
        values.sort_unstable();
        // a drawing the parts bring is one the document had (a copy keeps no number it had),
        // and raises the highest number given (never given again)
        let mut drawings: Vec<u64> = ooxml_word_model::ids::drawing_numbers(&roots)
            .into_iter()
            .filter(|n| !self.ids().had_drawing(*n))
            .collect();
        drawings.sort_unstable();
        let highest = drawings
            .last()
            .copied()
            .filter(|n| *n > self.ids().highest_drawing())
            .unwrap_or(0);
        let adds = !values.is_empty() || !drawings.is_empty();
        adds.then(|| {
            Change::Ids(IdsChange {
                values,
                highest_drawing: highest,
                drawings,
                ..IdsChange::default()
            })
        })
    }
}

/// Parts to put (`Some(bytes)`) or remove (`None`), by name, in one step.
pub struct PartUpdate<'a> {
    parts: Vec<(String, Option<Arc<[u8]>>)>,
    limits: Limits,
    cancel: &'a dyn Cancel,
}

/// What an update did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Updated {
    /// The version after it (the one it ran on when it changed nothing).
    pub version: u64,
    /// Whether it made a version.
    pub changed: bool,
    /// How many times it was prepared (more than one after a conflict).
    pub attempts: u32,
}

impl<'a> PartUpdate<'a> {
    /// An update of `parts`, parsed within `limits`, stopping when `cancel` says so.
    #[must_use]
    pub fn new(parts: Vec<(String, Option<Arc<[u8]>>)>, limits: Limits, cancel: &'a dyn Cancel) -> Self {
        Self { parts, limits, cancel }
    }
}

/// `new`, sharing with `old` (the part's tree in the version before) every subtree the two
/// have alike: `old` with the differences applied, by path copying; `new` itself where
/// there is no `old` or the two differ at the root.
fn shared_with(old: Option<Tree>, new: Tree) -> Tree {
    let Some(old) = old else { return new };
    let mut shared = old;
    for patch in ooxml_xml::diff(&shared, &new) {
        match patch.apply(&shared) {
            Ok(next) => shared = next,
            Err(_) => return new,
        }
    }
    new.with_root(std::sync::Arc::clone(shared.root()))
}

/// Whether a part's bytes are XML by its name (`.xml`, `.rels`, `.vml`).
fn is_xml(part: &str) -> bool {
    let lower = part.to_ascii_lowercase();
    [".xml", ".rels", ".vml"].iter().any(|e| lower.ends_with(e))
}

impl<D: Bridged> Command<D> for PartUpdate<'_>
where
    SessionError: From<Error<D>>,
{
    type Out = ();
    type Error = SessionError;

    fn validate(&self, doc: &D) -> Result<(), SessionError> {
        for (part, bytes) in &self.parts {
            let name = normalize(part);
            if name.is_empty() || name.ends_with('/') {
                return Err(SessionError::PartName(part.clone()));
            }
            if bytes.is_none() && !doc.package().has_part(&name) {
                return Err(SessionError::Package(ooxml_opc::OpcError::NoPart(name)));
            }
        }
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, D>) -> Result<(), SessionError> {
        // a commit of parts is exclusive: it holds only on the version it was made on
        tx.read_everything();
        let mut trees = Vec::new();
        for (part, bytes) in &self.parts {
            self.cancel.check()?;
            let name = normalize(part);
            let change = match bytes {
                None => D::remove(name),
                Some(bytes) if is_xml(&name) => {
                    let tree = ooxml_xml::parse(bytes, &self.limits.xml).map_err(|error| {
                        SessionError::Package(ooxml_opc::OpcError::Xml {
                            part: name.clone(),
                            error,
                        })
                    })?;
                    let tree = shared_with(tx.doc().package().tree(&name).ok().flatten(), tree);
                    trees.push(tree.clone());
                    D::put_tree(name, tree)
                }
                Some(bytes) => D::put_bytes(name, Arc::clone(bytes)),
            };
            tx.write(change)?;
        }
        let trees: Vec<&Tree> = trees.iter().collect();
        if let Some(ledger) = tx.doc().ledger(&trees) {
            tx.write(ledger)?;
        }
        Ok(())
    }
}
