//! Owns: [`PackageDoc`], a document as the store keeps it until the format models exist (a
//! version of its OPC package), and [`PartChange`], one primitive of a change set: a part
//! put, removed, or restored exactly as another version has it (what undo applies).
//!
//! Must not: know a format's parts; read or parse a part beyond what a change carries.
//! Keys are part names as OPC normalises them.

use ooxml_opc::name::normalize;
use ooxml_opc::{OpcError, Package};
use ooxml_store::{DocModel, KeySet};
use ooxml_xml::Tree;
use std::sync::Arc;

/// A document's package at one version.
#[derive(Debug, Clone)]
pub struct PackageDoc {
    package: Package,
}

impl PackageDoc {
    /// A document whose first version is `package`.
    #[must_use]
    pub fn new(package: Package) -> Self {
        Self { package }
    }

    /// The package at this version.
    #[must_use]
    pub fn package(&self) -> &Package {
        &self.package
    }
}

/// What a put part holds.
#[derive(Debug, Clone)]
pub enum PartContent {
    /// A parsed XML part (written in the write contract's form on save).
    Tree(Tree),
    /// Bytes, kept as given.
    Bytes(Arc<[u8]>),
}

/// One change to a package.
#[derive(Debug, Clone)]
pub enum PartChange {
    /// The part `part` holds `content` (a new part goes at the end of the archive).
    Put {
        /// The part's name.
        part: String,
        /// Its content.
        content: PartContent,
    },
    /// The part `part` is removed.
    Remove {
        /// The part's name.
        part: String,
    },
    /// The part `part` is as `from` has it: content, place and archive entry, or absent.
    Restore {
        /// The part's name.
        part: String,
        /// The version to take it from.
        from: Package,
    },
}

impl PartChange {
    /// The part it changes, normalised.
    #[must_use]
    pub fn part(&self) -> String {
        match self {
            PartChange::Put { part, .. } | PartChange::Remove { part } | PartChange::Restore { part, .. } => {
                normalize(part)
            }
        }
    }
}

/// Why a change could not be applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    /// The package refused it.
    #[error(transparent)]
    Package(#[from] OpcError),
}

impl DocModel for PackageDoc {
    type Key = String;
    type Change = PartChange;
    type Error = ModelError;

    fn apply(&mut self, change: &PartChange) -> Result<(), ModelError> {
        self.package = match change {
            PartChange::Put {
                part,
                content: PartContent::Tree(tree),
            } => self.package.with_new_tree(part, tree.clone()),
            PartChange::Put {
                part,
                content: PartContent::Bytes(bytes),
            } => self.package.with_bytes(part, Arc::clone(bytes)),
            PartChange::Remove { part } => self.package.without(part)?,
            PartChange::Restore { part, from } => self.package.restored(part, from)?,
        };
        Ok(())
    }

    fn inverse(&self, change: &PartChange) -> Result<PartChange, ModelError> {
        Ok(PartChange::Restore {
            part: change.part(),
            from: self.package.clone(),
        })
    }

    fn agrees(&self, other: &Self, keys: &KeySet<String>) -> bool {
        let same = |name: &str| match (self.package.part(name), other.package.part(name)) {
            (None, None) => true,
            (Some(a), Some(b)) => a.same(b),
            _ => false,
        };
        match keys.keys() {
            Some(keys) => keys.iter().all(|k| same(k)),
            None => {
                let names = self.package.part_names();
                names == other.package.part_names() && names.iter().all(|n| same(n))
            }
        }
    }

    fn written(change: &PartChange, into: &mut KeySet<String>) {
        into.insert(change.part());
    }
}
