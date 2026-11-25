//! Owns: a commit of parts given as bytes ([`crate::Session::update`], a version a test makes
//! outside the tools): the parts, parsed here and committed as one transaction and one
//! history step ([`PartUpdate`], a store [`Command`]). A part parsed again shares with the version
//! before every subtree that did not change (step 4: the two trees diffed structurally and
//! the differences applied to the old one by path copying), so what is kept per subtree --
//! block ids, a reading's facts, a layout's measures -- is found again by pointer, as after
//! an edit made here.
//!
//! The update reads the whole document (section 13, step 5): it commits only on the version
//! it was prepared on. The changes are the document's holder's ([`Holds`]): into a Word
//! document it also adds to the id ledger every id value the parts hold that the ledger
//! lacks, so an id the parts brought is never given again.
//!
//! Must not: interpret a part's content beyond parsing its XML and reading its ids, or run
//! anything with effects outside its transaction (a conflict prepares it again).

use crate::stored::Holds;
use crate::SessionError;
use ooxml_base::{Cancel, Limits};
use ooxml_opc::name::normalize;
use ooxml_store::{Command, DocModel, Txn};
use ooxml_xml::Tree;
use std::marker::PhantomData;
use std::sync::Arc;

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

/// `update` as a command on the model `D` its holder `H` keeps: the changes `H` writes.
pub(crate) struct Parts<'u, 'a, D, H> {
    pub(crate) update: &'u PartUpdate<'a>,
    pub(crate) model: PhantomData<fn() -> (D, H)>,
}

impl<D: DocModel, H: Holds<D>> Command<D> for Parts<'_, '_, D, H> {
    type Out = ();
    type Error = SessionError;

    fn validate(&self, doc: &D) -> Result<(), SessionError> {
        for (part, bytes) in &self.update.parts {
            let name = normalize(part);
            if name.is_empty() || name.ends_with('/') {
                return Err(SessionError::PartName(part.clone()));
            }
            if bytes.is_none() && !H::package(doc).has_part(&name) {
                return Err(SessionError::Package(ooxml_opc::OpcError::NoPart(name)));
            }
        }
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, D>) -> Result<(), SessionError> {
        // a commit of parts is exclusive: it holds only on the version it was made on
        tx.read_everything();
        let mut trees = Vec::new();
        for (part, bytes) in &self.update.parts {
            self.update.cancel.check()?;
            let name = normalize(part);
            let change = match bytes {
                None => H::remove(name),
                Some(bytes) if is_xml(&name) => {
                    let tree = ooxml_xml::parse(bytes, &self.update.limits.xml).map_err(|error| {
                        SessionError::Package(ooxml_opc::OpcError::Xml {
                            part: name.clone(),
                            error,
                        })
                    })?;
                    let tree = shared_with(H::package(tx.doc()).tree(&name).ok().flatten(), tree);
                    trees.push(tree.clone());
                    H::put_tree(name, tree)
                }
                Some(bytes) => H::put_bytes(name, Arc::clone(bytes)),
            };
            tx.write(change)?;
        }
        let trees: Vec<&Tree> = trees.iter().collect();
        if let Some(ledger) = H::ledger(tx.doc(), &trees) {
            tx.write(ledger)?;
        }
        Ok(())
    }
}
