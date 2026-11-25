//! Owns: [`WordDoc`], a Word document as the store keeps it -- a version of its package, its
//! ids ([`Ids`]), the version's reading as docx-agent reads it (memoised with the version),
//! the document-wide caches every version shares, and what the transaction that made the
//! version drew and spaced. What a change is: [`change`]; how a version takes one: [`model`].
//!
//! Must not: decide an edit (commands do, through the sink), read the clock, or hold global
//! state: caches belong to the document ([`Shared`]).

mod change;
mod model;

pub use change::{Change, Draw, DrawFrom, Key, ModelError};

use crate::pool::{EarlierIds, Ids, IdsChange, Sequences};
use crate::validate::{Facts, Tally};
use crate::EditError;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::{drawing_numbers, load_state, used_long_hex, BlockId, LoadedState};
use ooxml_xml::{Patch, PatchError, Tree};
use std::sync::{Arc, Mutex, PoisonError};

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
    /// Tables' shapes as Accept All leaves them, memoised by content.
    pub(crate) accepted: crate::facts::AcceptedShapes,
}

/// A Word document at one version.
#[derive(Debug, Clone)]
pub struct WordDoc {
    package: Package,
    ids: Ids,
    reading: Arc<Mutex<Lazy<Docx>>>,
    tally: Arc<Mutex<Lazy<Tally>>>,
    shared: Arc<Shared>,
    /// What the transaction that made this version drew for its stand-ins.
    drawn: crate::Drawn,
    /// How many spaces the transaction that made this version spaced ([`Change::Spaced`]).
    spaced: usize,
    /// Whether the transaction writes its text as given ([`Setup::as_given`]).
    as_given: bool,
}

/// How a run's transaction writes ([`ooxml_store::DocModel::set_up`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Setup {
    /// The text written as given: none of the spacing its language wants (docx-agent's
    /// `docx_format(typography=False)`); else spaced by its language when the edit ends.
    pub as_given: bool,
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
        let kept = Arc::new(ooxml_word_model::docx::Kept::new(Arc::clone(&budget)));
        let docx = Arc::new(Docx::open_keeping(&package, kept)?);
        Ok(WordDoc::of_docx(package, docx, budget, earlier))
    }

    /// The document whose first version is `package`, opened again with the id state saved
    /// beside its bytes (`docx-ids.1.`, docx-agent's round 38), read against its story
    /// parts as they are parsed once: the ids it had and what the state recorded beside
    /// them (the versions, each retired id and who retired it).
    ///
    /// # Errors
    /// As [`WordDoc::open`]; [`EditError::IdState`] when the state is not these bytes'.
    pub fn open_with_state(
        package: Package,
        budget: Arc<dyn CacheBudget>,
        state: &str,
    ) -> Result<(WordDoc, LoadedState), EditError> {
        let kept = Arc::new(ooxml_word_model::docx::Kept::new(Arc::clone(&budget)));
        let docx = Arc::new(Docx::open_keeping(&package, kept)?);
        let loaded = {
            let roots: Vec<&ooxml_xml::Element> = docx.parts.iter().map(|p| &**p.root()).collect();
            load_state(state, &roots)?
        };
        let earlier = EarlierIds::of_state(&loaded);
        Ok((WordDoc::of_docx(package, docx, budget, &earlier), loaded))
    }

    fn of_docx(package: Package, docx: Arc<Docx>, budget: Arc<dyn CacheBudget>, earlier: &EarlierIds) -> WordDoc {
        let roots: Vec<&ooxml_xml::Element> = docx.parts.iter().map(|p| &**p.root()).collect();
        let ids = Ids::opened(used_long_hex(&roots), drawing_numbers(&roots)).with_earlier(earlier);
        WordDoc {
            package,
            ids,
            reading: Arc::new(Mutex::new(Lazy::Ready(docx))),
            tally: Arc::new(Mutex::new(Lazy::Pending(None))),
            drawn: crate::Drawn::default(),
            spaced: 0,
            as_given: false,
            shared: Arc::new(Shared {
                sequences: Sequences::default(),
                fields: crate::fields::scan::FieldScans::new(Arc::clone(&budget)),
                accepted: crate::facts::AcceptedShapes::new(Arc::clone(&budget), crate::facts::accepted_weight),
                facts: Facts::new(budget),
            }),
        }
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

    /// Where the next draw from `seed` starts in its sequence ([`Ids::draws_from`]).
    #[must_use]
    pub fn draws_from(&self, seed: &str) -> usize {
        self.ids.draws_from(seed, &self.shared.sequences)
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
            drawn: self.drawn.clone(),
            spaced: self.spaced,
            as_given: self.as_given,
        }
    }

    /// What the transaction that made this version drew for its stand-ins: a reply naming
    /// ids the edit made resolves them through it.
    #[must_use]
    pub fn drawn(&self) -> &crate::Drawn {
        &self.drawn
    }

    /// How many spaces the transaction that made this version wrote or made U+00A0 for its
    /// text's language ([`Change::Spaced`]; 0 for a version no edit made).
    #[must_use]
    pub fn spaced(&self) -> usize {
        self.spaced
    }

    /// Whether the transaction working on this version writes its text as given, without
    /// its language's spacing ([`Setup::as_given`]).
    #[must_use]
    pub fn as_given(&self) -> bool {
        self.as_given
    }

    /// `patch` with `draws` made from this version's ledger: the values written into the one
    /// element it inserts, and the change that adds them to the ledger.
    fn drawn_patch(&self, patch: &Patch, draws: &[Draw]) -> Result<(Patch, IdsChange), ModelError> {
        debug_assert!(
            !crate::drawn::uncovered(patch.inserted(), draws, &self.drawn),
            "a stand-in in an id attribute no draw writes"
        );
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
                DrawFrom::Drawn(stand_in) => self.drawn.get(stand_in).ok_or_else(stale)?.to_owned(),
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

    /// `key` as this version drew it: a block key named by a stand-in its transaction drew,
    /// named by the value drawn; any other key as it is.
    fn resolved(&self, key: &Key) -> Key {
        match key {
            Key::Block { part, id } if !self.drawn.is_empty() => {
                let resolved = self.drawn.resolve(id);
                if *resolved == **id {
                    key.clone()
                } else {
                    Key::Block {
                        part: part.clone(),
                        id: BlockId::from(resolved),
                    }
                }
            }
            _ => key.clone(),
        }
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
