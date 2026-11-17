//! Owns: [`WordDoc`], a Word document as the store keeps it -- a version of its package, its
//! ids ([`Ids`]), the version's reading as docx-agent reads it (memoised with the version)
//! and the document-wide caches every version shares -- with what a transaction reads and
//! writes ([`Key`]) and the primitives of a change set ([`Change`]): tree patches per part,
//! parts put or removed, ledger additions.
//!
//! Must not: decide an edit (commands do, through the sink), read the clock, or hold global
//! state: caches belong to the document ([`Shared`]).

use crate::pool::{Ids, IdsChange, Sequences};
use crate::validate::Facts;
use crate::EditError;
use ooxml_derive::CacheBudget;
use ooxml_opc::{name::normalize, OpcError, Package};
use ooxml_store::{DocModel, KeySet};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::used_long_hex;
use ooxml_xml::{HeadPatch, Patch, PatchError, Tree};
use std::sync::{Arc, Mutex, PoisonError};

/// What a transaction reads and a change writes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// A paragraph, table or row of a story part, by its durable id (`p:1A2B3C4D`).
    Block {
        /// The part.
        part: Arc<str>,
        /// The block's id.
        id: Arc<str>,
    },
    /// A whole part (a part without durable ids, a part put or removed).
    Part(Arc<str>),
    /// A part's root element's head: its namespace declarations and attributes.
    Root(Arc<str>),
    /// An id sequence (a seed): drawing from it.
    Seq(Arc<str>),
    /// The annotation ids in use (revisions, bookmarks, comments): allocating one.
    Annotations,
}

/// One primitive of a change set.
#[derive(Debug, Clone)]
pub enum Change {
    /// A splice of an element's children in a part's tree.
    Splice {
        /// The part.
        part: Arc<str>,
        /// The patch.
        patch: Patch,
        /// The keys it writes.
        keys: Arc<[Key]>,
    },
    /// An element's head changed in place in a part's tree.
    Head {
        /// The part.
        part: Arc<str>,
        /// The patch.
        patch: HeadPatch,
        /// The keys it writes.
        keys: Arc<[Key]>,
    },
    /// A part put in place (a new part, or one replaced whole).
    Put {
        /// The part.
        part: Arc<str>,
        /// Its tree.
        tree: Tree,
    },
    /// A part as another version of the package has it, or absent there (what undo puts
    /// back).
    Restore {
        /// The part.
        part: Arc<str>,
        /// The version to take it from.
        from: Package,
    },
    /// Ids added to the ledger (never taken back: an id given is never given again).
    Ids(IdsChange),
}

/// Why a change could not be applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    /// The package refused it.
    #[error(transparent)]
    Package(#[from] OpcError),
    /// A patch did not apply or could not be rebased.
    #[error(transparent)]
    Patch(#[from] PatchError),
    /// A tree change names a part the package does not have.
    #[error("no part {0}")]
    NoPart(String),
}

/// A version's reading as docx-agent reads it, made on first use from the previous
/// version's (unchanged story parts are not read again).
#[derive(Debug)]
enum Reading {
    Pending(Option<Arc<Docx>>),
    Ready(Arc<Docx>),
}

/// What every version of one document shares: caches that belong to the document.
#[derive(Debug)]
pub struct Shared {
    /// The id sequences drawn from.
    pub(crate) sequences: Sequences,
    /// Per-block facts, memoised by content.
    pub(crate) facts: Facts,
}

/// A Word document at one version.
#[derive(Debug, Clone)]
pub struct WordDoc {
    package: Package,
    ids: Ids,
    reading: Arc<Mutex<Reading>>,
    shared: Arc<Shared>,
}

impl WordDoc {
    /// The document whose first version is `package`: its ids read from every story part,
    /// its caches within `budget`.
    ///
    /// # Errors
    /// [`EditError`] when the package is not a Word document a reading can open.
    pub fn open(package: Package, budget: Arc<dyn CacheBudget>) -> Result<WordDoc, EditError> {
        let docx = Arc::new(Docx::open(&package)?);
        let roots: Vec<&ooxml_xml::Element> = docx.parts.iter().map(|p| &**p.root()).collect();
        let ids = Ids::opened(used_long_hex(&roots));
        Ok(WordDoc {
            package,
            ids,
            reading: Arc::new(Mutex::new(Reading::Ready(docx))),
            shared: Arc::new(Shared {
                sequences: Sequences::default(),
                facts: Facts::new(budget),
            }),
        })
    }

    /// The package.
    #[must_use]
    pub fn package(&self) -> &Package {
        &self.package
    }

    /// The ids.
    #[must_use]
    pub fn ids(&self) -> &Ids {
        &self.ids
    }

    /// The `count` values the next draw from `seed` would give (nothing is drawn).
    #[must_use]
    pub fn next_ids(&self, seed: &str, count: usize) -> Vec<String> {
        self.ids.draw(seed, count, &self.shared.sequences).0
    }

    /// What the document's versions share.
    #[must_use]
    pub fn shared(&self) -> &Shared {
        &self.shared
    }

    /// This version as docx-agent's reading tools read it.
    ///
    /// # Errors
    /// [`EditError`] when a story part is not XML.
    pub fn docx(&self) -> Result<Arc<Docx>, EditError> {
        let mut reading = self.reading.lock().unwrap_or_else(PoisonError::into_inner);
        let docx = match &*reading {
            Reading::Ready(docx) => return Ok(Arc::clone(docx)),
            Reading::Pending(before) => Arc::new(Docx::open_after(&self.package, before.as_deref())?),
        };
        *reading = Reading::Ready(Arc::clone(&docx));
        Ok(docx)
    }

    /// A part's tree.
    ///
    /// # Errors
    /// When the part is not XML.
    pub fn tree(&self, part: &str) -> Result<Option<Tree>, EditError> {
        Ok(self.package.tree(part)?)
    }

    /// The next version: `package` and `ids`, its reading made from this one's.
    fn succeed(&self, package: Package, ids: Ids) -> WordDoc {
        let before = match &*self.reading.lock().unwrap_or_else(PoisonError::into_inner) {
            Reading::Ready(docx) => Some(Arc::clone(docx)),
            Reading::Pending(before) => before.clone(),
        };
        WordDoc {
            package,
            ids,
            reading: Arc::new(Mutex::new(Reading::Pending(before))),
            shared: Arc::clone(&self.shared),
        }
    }

    fn with_tree(
        &self,
        part: &str,
        edit: impl FnOnce(&Tree) -> Result<Tree, PatchError>,
    ) -> Result<Package, ModelError> {
        let tree = self
            .package
            .tree(part)?
            .ok_or_else(|| ModelError::NoPart(part.to_owned()))?;
        Ok(self.package.with_tree(part, edit(&tree)?)?)
    }
}

impl Change {
    /// The part it changes, if it changes one.
    #[must_use]
    pub fn part(&self) -> Option<&str> {
        match self {
            Change::Splice { part, .. }
            | Change::Head { part, .. }
            | Change::Put { part, .. }
            | Change::Restore { part, .. } => Some(part),
            Change::Ids(_) => None,
        }
    }
}

impl DocModel for WordDoc {
    type Key = Key;
    type Change = Change;
    type Error = ModelError;

    fn apply(&mut self, change: &Change) -> Result<(), ModelError> {
        let (package, ids) = match change {
            Change::Splice { part, patch, .. } => (self.with_tree(part, |t| patch.apply(t))?, self.ids.clone()),
            Change::Head { part, patch, .. } => (self.with_tree(part, |t| patch.apply(t))?, self.ids.clone()),
            Change::Put { part, tree } => (self.package.with_new_tree(part, tree.clone()), self.ids.clone()),
            Change::Restore { part, from } => (self.package.restored(part, from)?, self.ids.clone()),
            Change::Ids(added) => {
                let mut ids = self.ids.clone();
                ids.apply(added);
                (self.package.clone(), ids)
            }
        };
        *self = self.succeed(package, ids);
        Ok(())
    }

    fn inverse(&self, change: &Change) -> Result<Change, ModelError> {
        Ok(match change {
            Change::Splice { part, patch, keys } => Change::Splice {
                part: Arc::clone(part),
                patch: patch.inverse(),
                keys: Arc::clone(keys),
            },
            Change::Head { part, patch, keys } => Change::Head {
                part: Arc::clone(part),
                patch: patch.inverse(),
                keys: Arc::clone(keys),
            },
            Change::Put { part, .. } | Change::Restore { part, .. } => Change::Restore {
                part: Arc::clone(part),
                from: self.package.clone(),
            },
            // ids given are never given again: nothing to take back
            Change::Ids(_) => Change::Ids(IdsChange::default()),
        })
    }

    fn agrees(&self, other: &Self, keys: &KeySet<Key>) -> bool {
        let same_part = |name: &str| match (self.package.part(name), other.package.part(name)) {
            (None, None) => true,
            (Some(a), Some(b)) => a.same(b),
            _ => false,
        };
        match keys.keys() {
            // a block is compared through its part: conservative, never a false agreement
            Some(keys) => keys.iter().all(|key| match key {
                Key::Block { part, .. } | Key::Part(part) | Key::Root(part) => same_part(part),
                Key::Seq(_) | Key::Annotations => true,
            }),
            None => {
                let names = self.package.part_names();
                names == other.package.part_names() && names.iter().all(|n| same_part(n))
            }
        }
    }

    fn written(change: &Change, into: &mut KeySet<Key>) {
        match change {
            Change::Splice { keys, .. } | Change::Head { keys, .. } => {
                for key in keys.iter() {
                    into.insert(key.clone());
                }
            }
            Change::Put { part, .. } | Change::Restore { part, .. } => {
                into.insert(Key::Part(Arc::from(normalize(part))))
            }
            Change::Ids(added) => {
                for (seed, _) in &added.cursors {
                    into.insert(Key::Seq(Arc::from(seed.as_str())));
                }
            }
        }
    }

    fn rebase(change: &Change, past: &Change) -> Result<Change, ModelError> {
        if change.part().is_none() || change.part() != past.part() {
            return Ok(change.clone());
        }
        Ok(match (change, past) {
            (Change::Splice { part, patch, keys }, Change::Splice { patch: earlier, .. }) => Change::Splice {
                part: Arc::clone(part),
                patch: patch.rebase(earlier)?,
                keys: Arc::clone(keys),
            },
            (Change::Splice { part, patch, keys }, Change::Head { patch: earlier, .. }) => Change::Splice {
                part: Arc::clone(part),
                patch: patch.rebase_head(earlier)?,
                keys: Arc::clone(keys),
            },
            (Change::Head { part, patch, keys }, Change::Splice { patch: earlier, .. }) => Change::Head {
                part: Arc::clone(part),
                patch: patch.rebase(earlier)?,
                keys: Arc::clone(keys),
            },
            (Change::Head { part, patch, keys }, Change::Head { patch: earlier, .. }) => Change::Head {
                part: Arc::clone(part),
                patch: patch.rebase_head(earlier)?,
                keys: Arc::clone(keys),
            },
            // a part put or restored whole cannot be moved past another change of it
            _ => return Err(ModelError::Patch(PatchError::Conflict(Vec::new()))),
        })
    }
}
