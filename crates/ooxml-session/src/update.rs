//! Owns: the bridge's commit (section 13): a version's touched parts, given as bytes by the
//! owner of the document, parsed here and committed as one transaction and one history
//! step ([`PartUpdate`], a store [`Command`]). A part parsed again shares with the version
//! before every subtree that did not change (step 4: the two trees diffed structurally and
//! the differences applied to the old one by path copying), so what is kept per subtree --
//! block ids, a reading's facts, a layout's measures -- is found again by pointer, as after
//! an edit made here.
//!
//! Must not: interpret a part's content beyond parsing its XML, or run anything with
//! effects outside its transaction (a conflict prepares it again).

use crate::model::{PackageDoc, PartChange, PartContent};
use crate::SessionError;
use ooxml_base::{Cancel, Limits};
use ooxml_opc::name::normalize;
use ooxml_store::{Command, Txn};
use ooxml_xml::Tree;
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

impl Command<PackageDoc> for PartUpdate<'_> {
    type Out = ();
    type Error = SessionError;

    fn validate(&self, doc: &PackageDoc) -> Result<(), SessionError> {
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

    fn prepare(&self, tx: &mut Txn<'_, PackageDoc>) -> Result<(), SessionError> {
        for (part, bytes) in &self.parts {
            self.cancel.check()?;
            let name = normalize(part);
            let change = match bytes {
                None => {
                    // a removal depends on the part being there
                    tx.read(name.clone());
                    PartChange::Remove { part: name }
                }
                Some(bytes) if is_xml(&name) => {
                    let tree = ooxml_xml::parse(bytes, &self.limits.xml).map_err(|error| {
                        SessionError::Package(ooxml_opc::OpcError::Xml {
                            part: name.clone(),
                            error,
                        })
                    })?;
                    let tree = shared_with(tx.doc().package().tree(&name).ok().flatten(), tree);
                    PartChange::Put {
                        part: name,
                        content: PartContent::Tree(tree),
                    }
                }
                Some(bytes) => PartChange::Put {
                    part: name,
                    content: PartContent::Bytes(Arc::clone(bytes)),
                },
            };
            tx.write(change)?;
        }
        Ok(())
    }
}
