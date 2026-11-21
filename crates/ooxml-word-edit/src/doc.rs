//! Owns: [`WordDoc`], a Word document as the store keeps it -- a version of its package, its
//! ids ([`Ids`]), the version's reading as docx-agent reads it (memoised with the version)
//! and the document-wide caches every version shares -- with what a transaction reads and
//! writes ([`Key`]) and the primitives of a change set ([`Change`]): tree patches per part,
//! parts put or removed, ledger additions.
//!
//! Must not: decide an edit (commands do, through the sink), read the clock, or hold global
//! state: caches belong to the document ([`Shared`]).

use crate::pool::{EarlierIds, Ids, IdsChange, Sequences};
use crate::validate::{Facts, Tally};
use crate::EditError;
use ooxml_derive::CacheBudget;
use ooxml_opc::{name::normalize, OpcError, Package};
use ooxml_store::{DocModel, KeySet};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::{drawing_numbers, used_long_hex};
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
    /// Who the people part lists.
    People,
    /// What a part's paragraphs show a reader of their style uses: each paragraph with text
    /// or in a list, in order, with its paragraph style, its list and whether its text is
    /// blank (the body style a copy maps to, the list a copied list continues). Written
    /// where a change alters one (`sink::style_use`), and with the part put or removed.
    StyleUse(Arc<str>),
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
        /// Ids drawn from the ledger where the change is applied, written into the
        /// inserted element (the ledger merged at commit: a draw is not a conflict).
        draws: Arc<[Draw]>,
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
    /// A part put in place as bytes, kept as given (media, or any part a bridged tool wrote
    /// that is not XML).
    Bytes {
        /// The part.
        part: Arc<str>,
        /// Its bytes.
        bytes: Arc<[u8]>,
    },
    /// A part removed (a relationships part left empty, a part nothing uses any more).
    Remove {
        /// The part.
        part: Arc<str>,
    },
    /// Parts as another version of the package has them, or absent there (what undo puts
    /// back).
    Restore {
        /// The parts.
        parts: Arc<[Arc<str>]>,
        /// The version to take them from.
        from: Package,
    },
    /// The people part kept, where the change is applied (docx-agent's `Stamp.finish`):
    /// `add` listed (the part made when there is none), then `tidy` dropped when nothing
    /// of theirs is left in use (the part with it once nobody is listed). Applied at
    /// commit, so two writers listing themselves do not conflict.
    People {
        /// The author to list.
        add: Option<Arc<str>>,
        /// The author to drop if none of their revisions or comments is left.
        tidy: Option<Arc<str>>,
        /// The keys it writes (the parts it may change).
        keys: Arc<[Key]>,
    },
    /// Ids added to the ledger (never taken back: an id given is never given again).
    Ids(IdsChange),
    /// Where the edit's revision ids start: one above the largest annotation id in use in
    /// the version it is applied to (docx-agent's `Stamp`, made as the edit begins
    /// recording). The draws after it ([`DrawFrom::Annotation`]) count from there.
    Number,
}

/// Where a drawn value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawFrom {
    /// The next value of a seed's sequence the ledger lacks (paraIds, textIds).
    Seed(Arc<str>),
    /// An annotation id: the edit's first ([`Change::Number`]) plus this offset (its
    /// revision records, numbered in the order it made them).
    Annotation(u32),
}

/// An id drawn when a splice is applied, into an attribute of the element it inserts: the
/// value is the one the version it is applied to gives next, so a commit rebased past
/// others draws what a serial run would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draw {
    /// Where the value comes from.
    pub from: DrawFrom,
    /// Which of the splice's inserted nodes it goes into (0 for a splice of one).
    pub node: u32,
    /// The element's place below that node (empty: the node itself).
    pub at: Vec<u32>,
    /// The attribute the value goes into.
    pub attribute: ooxml_xml::Name,
}

/// `element` with `name` set to `value` on the element at `at` below it.
fn set_below(
    element: &Arc<ooxml_xml::Element>,
    at: &[u32],
    name: &ooxml_xml::Name,
    value: &str,
) -> Option<Arc<ooxml_xml::Element>> {
    let Some((&k, rest)) = at.split_first() else {
        let head = crate::stamping::with_attribute(element.head(), name, value);
        return Some(Arc::new(ooxml_xml::Element::from_head(
            Arc::new(head),
            element.children().clone(),
        )));
    };
    let child = element.children().get(k as usize)?.as_element()?;
    let edited = set_below(child, rest, name, value)?;
    let children = element
        .children()
        .splice(k as usize..k as usize + 1, vec![ooxml_xml::Node::Element(edited)])?;
    Some(Arc::new(element.with_children(children)))
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
    /// The version could not be read as a Word document.
    #[error("{0}")]
    Unreadable(String),
}

/// What a version derives once, on first use, from the latest earlier version's (its
/// reading as docx-agent reads it, its validation tally): unchanged parts are not read
/// again.
#[derive(Debug)]
enum Lazy<T> {
    /// Not made yet: the earlier version's, to make it from.
    Pending(Option<Arc<T>>),
    Ready(Arc<T>),
}

impl<T> Lazy<T> {
    /// What the next version makes its own from.
    fn handed_on(&self) -> Option<Arc<T>> {
        match self {
            Lazy::Ready(made) => Some(Arc::clone(made)),
            Lazy::Pending(before) => before.clone(),
        }
    }
}

/// The value of `cell`, made by `make` (given the earlier version's) on first use.
fn derived<T, E>(cell: &Mutex<Lazy<T>>, make: impl FnOnce(Option<&T>) -> Result<T, E>) -> Result<Arc<T>, E> {
    let mut slot = cell.lock().unwrap_or_else(PoisonError::into_inner);
    let made = match &*slot {
        Lazy::Ready(made) => return Ok(Arc::clone(made)),
        Lazy::Pending(before) => Arc::new(make(before.as_deref())?),
    };
    *slot = Lazy::Ready(Arc::clone(&made));
    Ok(made)
}

/// What every version of one document shares: caches that belong to the document.
#[derive(Debug)]
pub struct Shared {
    /// The id sequences drawn from.
    pub(crate) sequences: Sequences,
    /// Per-block facts, memoised by content.
    pub(crate) facts: Facts,
    /// Per-block fields, memoised by content.
    pub(crate) fields: crate::fields::scan::FieldScans,
}

/// A Word document at one version.
#[derive(Debug, Clone)]
pub struct WordDoc {
    package: Package,
    ids: Ids,
    reading: Arc<Mutex<Lazy<Docx>>>,
    tally: Arc<Mutex<Lazy<Tally>>>,
    shared: Arc<Shared>,
}

impl WordDoc {
    /// The document whose first version is `package`: its ids read from every story part,
    /// its caches within `budget`.
    ///
    /// # Errors
    /// [`EditError`] when the package is not a Word document a reading can open.
    pub fn open(package: Package, budget: Arc<dyn CacheBudget>) -> Result<WordDoc, EditError> {
        WordDoc::open_with(package, budget, &EarlierIds::default())
    }

    /// The document whose first version is `package`, opened again with the ids it had
    /// before it was saved (`earlier`) beside those its parts hold.
    ///
    /// # Errors
    /// As [`WordDoc::open`].
    pub fn open_with(
        package: Package,
        budget: Arc<dyn CacheBudget>,
        earlier: &EarlierIds,
    ) -> Result<WordDoc, EditError> {
        let docx = Arc::new(Docx::open(&package)?);
        let roots: Vec<&ooxml_xml::Element> = docx.parts.iter().map(|p| &**p.root()).collect();
        let ids = Ids::opened(used_long_hex(&roots), drawing_numbers(&roots)).with_earlier(earlier);
        Ok(WordDoc {
            package,
            ids,
            reading: Arc::new(Mutex::new(Lazy::Ready(docx))),
            tally: Arc::new(Mutex::new(Lazy::Pending(None))),
            shared: Arc::new(Shared {
                sequences: Sequences::default(),
                fields: crate::fields::scan::FieldScans::new(Arc::clone(&budget)),
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

    /// The same, as a handle that outlives this version's borrow.
    pub(crate) fn shared_handle(&self) -> Arc<Shared> {
        Arc::clone(&self.shared)
    }

    /// This version as docx-agent's reading tools read it.
    ///
    /// # Errors
    /// [`EditError`] when a story part is not XML.
    pub fn docx(&self) -> Result<Arc<Docx>, EditError> {
        derived(&self.reading, |before| Ok(Docx::open_after(&self.package, before)?))
    }

    /// This version's validation facts, kept in place from the latest earlier version's.
    ///
    /// # Errors
    /// [`EditError`] when a story part is not XML.
    pub(crate) fn tally(&self) -> Result<Arc<Tally>, EditError> {
        derived(&self.tally, |before| Tally::of(self, before))
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
        let reading = self.reading.lock().unwrap_or_else(PoisonError::into_inner).handed_on();
        let tally = self.tally.lock().unwrap_or_else(PoisonError::into_inner).handed_on();
        WordDoc {
            package,
            ids,
            reading: Arc::new(Mutex::new(Lazy::Pending(reading))),
            tally: Arc::new(Mutex::new(Lazy::Pending(tally))),
            shared: Arc::clone(&self.shared),
        }
    }

    /// `patch` with `draws` made from this version's ledger: the values written into the one
    /// element it inserts, and the change that adds them to the ledger.
    fn drawn(&self, patch: &Patch, draws: &[Draw]) -> Result<(Patch, IdsChange), ModelError> {
        let stale = || {
            ModelError::Patch(PatchError::Stale {
                parent: patch.parent().0.clone(),
                start: patch.start(),
            })
        };
        let mut inserted = patch.inserted().to_vec();
        let mut ids = self.ids.clone();
        let mut added = IdsChange::default();
        for draw in draws {
            let value = match &draw.from {
                DrawFrom::Seed(seed) => {
                    let (values, change) = ids.draw(seed, 1, &self.shared.sequences);
                    ids.apply(&change);
                    added.values.extend(change.values);
                    added.cursors.extend(change.cursors);
                    values.into_iter().next().ok_or_else(stale)?
                }
                DrawFrom::Annotation(offset) => {
                    let first = self.ids.annotations().ok_or_else(stale)?;
                    (first + i64::from(*offset)).to_string()
                }
            };
            let slot = inserted.get_mut(draw.node as usize).ok_or_else(stale)?;
            let ooxml_xml::Node::Element(element) = slot else {
                return Err(stale());
            };
            *element = set_below(element, &draw.at, &draw.attribute, &value).ok_or_else(stale)?;
        }
        let patch = Patch::splice(
            patch.parent().clone(),
            patch.start(),
            patch.removed().to_vec(),
            inserted,
        );
        Ok((patch, added))
    }

    /// The package with the people part kept (see [`Change::People`]).
    fn people(&self, add: Option<&str>, tidy: Option<&str>) -> Result<Package, ModelError> {
        let unreadable = |e: EditError| ModelError::Unreadable(e.to_string());
        let main = self.docx().map_err(unreadable)?.main.clone();
        let mut package = self.package.clone();
        if let Some(author) = add {
            package = crate::people::with_person(&package, &main, author)?;
        }
        if let Some(author) = tidy {
            if !crate::validate::author_in_use(self, author).map_err(unreadable)? {
                package = crate::people::without_person(&package, &main, author)?;
            }
        }
        Ok(package)
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
            | Change::Bytes { part, .. }
            | Change::Remove { part } => Some(part),
            Change::Restore { .. } | Change::Ids(_) | Change::Number | Change::People { .. } => None,
        }
    }
}

impl DocModel for WordDoc {
    type Key = Key;
    type Change = Change;
    type Error = ModelError;

    fn apply(&mut self, change: &Change) -> Result<(), ModelError> {
        let (package, ids) = match change {
            Change::Splice { part, patch, draws, .. } if !draws.is_empty() => {
                let (patch, added) = self.drawn(patch, draws)?;
                let mut ids = self.ids.clone();
                ids.apply(&added);
                (self.with_tree(part, |t| patch.apply(t))?, ids)
            }
            Change::Splice { part, patch, .. } => (self.with_tree(part, |t| patch.apply(t))?, self.ids.clone()),
            Change::Head { part, patch, .. } => (self.with_tree(part, |t| patch.apply(t))?, self.ids.clone()),
            Change::Put { part, tree } => (self.package.with_new_tree(part, tree.clone()), self.ids.clone()),
            Change::Bytes { part, bytes } => (self.package.with_bytes(part, Arc::clone(bytes)), self.ids.clone()),
            Change::Remove { part } => (self.package.without(part)?, self.ids.clone()),
            Change::Restore { parts, from } => {
                let mut package = self.package.clone();
                for part in parts.iter() {
                    package = package.restored(part, from)?;
                }
                (package, self.ids.clone())
            }
            Change::People { add, tidy, .. } => (self.people(add.as_deref(), tidy.as_deref())?, self.ids.clone()),
            Change::Ids(added) => {
                let mut ids = self.ids.clone();
                ids.apply(added);
                (self.package.clone(), ids)
            }
            Change::Number => {
                let ceiling =
                    crate::validate::annotation_ceiling(self).map_err(|e| ModelError::Unreadable(e.to_string()))?;
                let mut ids = self.ids.clone();
                ids.number_from(ceiling + 1);
                (self.package.clone(), ids)
            }
        };
        *self = self.succeed(package, ids);
        Ok(())
    }

    fn inverse(&self, change: &Change) -> Result<Change, ModelError> {
        Ok(match change {
            Change::Splice {
                part,
                patch,
                keys,
                draws,
            } => {
                // what was inserted is what the draws made of it here
                let patch = if draws.is_empty() {
                    patch.clone()
                } else {
                    self.drawn(patch, draws)?.0
                };
                Change::Splice {
                    part: Arc::clone(part),
                    patch: patch.inverse(),
                    keys: Arc::clone(keys),
                    draws: Arc::from([]),
                }
            }
            Change::Head { part, patch, keys } => Change::Head {
                part: Arc::clone(part),
                patch: patch.inverse(),
                keys: Arc::clone(keys),
            },
            Change::Put { part, .. } | Change::Bytes { part, .. } | Change::Remove { part } => Change::Restore {
                parts: Arc::from([Arc::clone(part)]),
                from: self.package.clone(),
            },
            Change::Restore { parts, .. } => Change::Restore {
                parts: Arc::clone(parts),
                from: self.package.clone(),
            },
            Change::People { .. } => {
                let main = self
                    .docx()
                    .map_err(|e| ModelError::Unreadable(e.to_string()))?
                    .main
                    .clone();
                Change::Restore {
                    parts: crate::people::parts_written(&self.package, &main)?.into(),
                    from: self.package.clone(),
                }
            }
            // ids given are never given again: nothing to take back
            Change::Ids(_) | Change::Number => Change::Ids(IdsChange::default()),
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
                Key::Block { part, .. } | Key::Part(part) | Key::Root(part) | Key::StyleUse(part) => same_part(part),
                Key::Seq(_) | Key::Annotations | Key::People => true,
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
            Change::Put { part, .. } | Change::Bytes { part, .. } | Change::Remove { part } => {
                let part: Arc<str> = Arc::from(normalize(part));
                into.insert(Key::StyleUse(Arc::clone(&part)));
                into.insert(Key::Part(part));
            }
            Change::Restore { parts, .. } => {
                for part in parts.iter() {
                    let part: Arc<str> = Arc::from(normalize(part));
                    into.insert(Key::StyleUse(Arc::clone(&part)));
                    into.insert(Key::Part(part));
                }
            }
            Change::People { keys, .. } => {
                for key in keys.iter() {
                    into.insert(key.clone());
                }
            }
            Change::Ids(added) => {
                for (seed, _) in &added.cursors {
                    into.insert(Key::Seq(Arc::from(seed.as_str())));
                }
                if added.highest_drawing > 0 || !added.drawings.is_empty() {
                    into.insert(Key::Seq(Arc::from(crate::pool::DRAWING_NUMBERS)));
                }
            }
            // numbering revision records adds annotation ids: a transaction that read the
            // ids in use (to number past them where it is prepared) conflicts with it
            Change::Number => {
                into.insert(Key::Annotations);
            }
        }
    }

    fn kept(key: &Key) -> bool {
        // ids drawn stay drawn: undoing a step gives none of them back
        matches!(key, Key::Seq(_) | Key::Annotations)
    }

    fn carry(&mut self, head: &Self) {
        // the ledger is the document's: an undo or redo keeps every id given
        self.ids.carry(&head.ids);
    }

    fn settle(&self) {
        // the reading and the tally, each from the version before (the head's, settled):
        // what the next commit's draws and people change read where they land
        let _ = self.docx();
        let _ = self.tally();
    }

    fn merged(change: &Change, into: &mut KeySet<Key>) {
        match change {
            // revision ids drawn where the splice lands, the people part kept there
            Change::Splice { keys, .. } => {
                if keys.contains(&Key::Annotations) {
                    into.insert(Key::Annotations);
                }
            }
            Change::People { keys, .. } => {
                for key in keys.iter() {
                    into.insert(key.clone());
                }
            }
            // ids numbered from the version the change lands on: two edits numbering their
            // revision records give the serial result in either commit order
            Change::Number => {
                into.insert(Key::Annotations);
            }
            _ => {}
        }
    }

    fn rebase(change: &Change, past: &Change) -> Result<Change, ModelError> {
        // parts restored whole (an undo's inverse of a part put) cannot move past another
        // change of one of them, nor another change past them
        let restores = |restore: &Change, other: &Change| match (restore, other.part()) {
            (Change::Restore { parts, .. }, Some(part)) => parts.iter().any(|p| **p == *part),
            _ => false,
        };
        if restores(change, past) || restores(past, change) {
            return Err(ModelError::Patch(PatchError::Conflict(Vec::new())));
        }
        if change.part().is_none() || change.part() != past.part() {
            return Ok(change.clone());
        }
        Ok(match (change, past) {
            (
                Change::Splice {
                    part,
                    patch,
                    keys,
                    draws,
                },
                Change::Splice { patch: earlier, .. },
            ) => Change::Splice {
                part: Arc::clone(part),
                patch: patch.rebase(earlier)?,
                keys: Arc::clone(keys),
                draws: Arc::clone(draws),
            },
            (
                Change::Splice {
                    part,
                    patch,
                    keys,
                    draws,
                },
                Change::Head { patch: earlier, .. },
            ) => Change::Splice {
                part: Arc::clone(part),
                patch: patch.rebase_head(earlier)?,
                keys: Arc::clone(keys),
                draws: Arc::clone(draws),
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
