//! Owns: [`WordDoc`], a Word document as the store keeps it -- a version of its package, its
//! ids ([`Ids`]), the version's reading as docx-agent reads it (memoised with the version),
//! the document-wide caches every version shares, and what the transaction that made the
//! version drew and spaced. What a change is: [`change`]; how a version takes one: [`model`];
//! a transaction's splices applied where their ids came out otherwise: [`redrawn`].
//!
//! Must not: decide an edit (commands do, through the sink), read the clock, or hold global
//! state: caches belong to the document ([`Shared`]).

mod change;
mod model;
mod redrawn;

pub use change::{Change, Draw, DrawFrom, Key, ModelError};

use crate::pool::{EarlierIds, Ids, IdsChange, Sequences};
use crate::validate::{Facts, Tally};
use crate::EditError;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::{
    block_and_drawing_ids, drawing_numbers, load_state, BlockId, DocumentId, IdPool, LoadedState, PartsDelta,
    VersionDelta,
};
use ooxml_word_model::WordPackage;
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
        let head = ooxml_word_model::stamp::with_attribute(element.head(), name, value);
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

/// The value of the attribute `name` of the element at `at` below `element`, if it has one.
fn attribute_below<'e>(element: &'e ooxml_xml::Element, at: &[u32], name: &ooxml_xml::Name) -> Option<&'e str> {
    let Some((&k, rest)) = at.split_first() else {
        return element
            .attributes()
            .iter()
            .find(|a| a.name.as_ref() == name.as_ref())
            .map(|a| a.value.as_ref());
    };
    attribute_below(element.children().get(k as usize)?.as_element()?, rest, name)
}

/// What a version derives once, on first use, from the latest earlier version's (its
/// validation tally): unchanged parts are not read again.
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

/// A version's reading as docx-agent reads it, made on first use from the latest earlier
/// version's (the document's one chain of readings, [`Docx::open_after`]): unchanged parts
/// are not read again.
#[derive(Debug)]
enum Reading {
    /// Not made yet: the earlier version's, to make it from.
    Pending(Arc<Docx>),
    Ready(Arc<Docx>),
}

impl Reading {
    /// What the next version makes its own from.
    fn handed_on(&self) -> Arc<Docx> {
        match self {
            Reading::Ready(made) | Reading::Pending(made) => Arc::clone(made),
        }
    }
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
    /// The document's identity, whatever its version: what its store's version numbers are
    /// named with ([`ooxml_word_model::ids::VersionRef`]).
    document: DocumentId,
    /// Whether it opened as a strict Open XML document ([`ooxml_word_model::parts::strict`]):
    /// a property of the document, found once at open. No change applies to one.
    strict: bool,
}

/// A Word document at one version.
#[derive(Debug, Clone)]
pub struct WordDoc {
    package: WordPackage,
    ids: Ids,
    reading: Arc<Mutex<Reading>>,
    tally: Arc<Mutex<Lazy<Tally>>>,
    shared: Arc<Shared>,
    /// What the transaction that made this version drew for its stand-ins.
    drawn: crate::Drawn,
    /// What the transaction that made this version inserted, as prepared and as applied
    /// where its ids were drawn again ([`redrawn::Redrawn`]): read only by its own later
    /// changes' apply.
    redrawn: redrawn::Redrawn,
    /// How many spaces the transaction that made this version spaced ([`Change::Spaced`]).
    spaced: usize,
    /// Whether the transaction writes its text as given ([`Setup::as_given`]).
    as_given: bool,
    /// The package the transaction working on this version began on: an id it read with
    /// ids given at open and not written is fresh for the whole transaction
    /// ([`WordDoc::id_written`]).
    begun: Option<WordPackage>,
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
    /// each block without a durable id given one in memory ([`WordPackage::open`]), its
    /// caches within `budget`.
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
        let (package, pool) = WordPackage::open(package);
        let docx = Arc::new(Docx::open(&package, Arc::clone(&budget))?);
        let strict = ooxml_word_model::parts::strict(&package)?;
        Ok(WordDoc::of_docx(package, docx, budget, (&pool, strict), earlier))
    }

    /// The document whose first version is `package`, opened again with the id state saved
    /// beside its bytes (`docx-ids.1.`, docx-agent's round 38), read against its story
    /// parts as they are saved (the ids given at open are not the bytes'): the ids it had
    /// and what the state recorded beside them (the versions, each retired id and who
    /// retired it). An id the state retired that the open gave again (a state written with
    /// a document's stamping undone) is live, not retired.
    ///
    /// # Errors
    /// As [`WordDoc::open`]; [`EditError::IdState`] when the state is not these bytes'.
    pub fn open_with_state(
        package: Package,
        budget: Arc<dyn CacheBudget>,
        state: &str,
    ) -> Result<(WordDoc, LoadedState), EditError> {
        let (package, pool) = WordPackage::open(package);
        let docx = Arc::new(Docx::open(&package, Arc::clone(&budget))?);
        let mut loaded = {
            let mut saved = Vec::with_capacity(docx.parts.len());
            for part in &docx.parts {
                saved.extend(package.saved_tree(&part.part)?);
            }
            let roots: Vec<&ooxml_xml::Element> = saved.iter().map(|t| &**t.root()).collect();
            load_state(state, &roots)?
        };
        let roots: Vec<&ooxml_xml::Element> = docx.parts.iter().map(|p| &**p.root()).collect();
        let (live, _) = block_and_drawing_ids(&roots);
        loaded.gone.retain(|value, _| !live.contains(value));
        let earlier = EarlierIds::of_state(&loaded);
        let strict = ooxml_word_model::parts::strict(&package)?;
        Ok((
            WordDoc::of_docx(package, docx, budget, (&pool, strict), &earlier),
            loaded,
        ))
    }

    /// The first version: `package` read as `docx`, its ids the pool its blocks' ids were
    /// drawn from at open (`pool`) and what it had before (`earlier`); `strict`, whether it is
    /// a strict Open XML document.
    fn of_docx(
        package: WordPackage,
        docx: Arc<Docx>,
        budget: Arc<dyn CacheBudget>,
        (pool, strict): (&IdPool, bool),
        earlier: &EarlierIds,
    ) -> WordDoc {
        let roots: Vec<&ooxml_xml::Element> = docx.parts.iter().map(|p| &**p.root()).collect();
        let ids = Ids::opened(pool.values().clone(), drawing_numbers(&roots))
            .with_cursors(pool.cursors())
            .with_earlier(earlier);
        WordDoc {
            package,
            ids,
            reading: Arc::new(Mutex::new(Reading::Ready(docx))),
            tally: Arc::new(Mutex::new(Lazy::Pending(None))),
            drawn: crate::Drawn::default(),
            redrawn: redrawn::Redrawn::default(),
            spaced: 0,
            as_given: false,
            begun: None,
            shared: Arc::new(Shared {
                sequences: Sequences::default(),
                fields: crate::fields::scan::FieldScans::new(Arc::clone(&budget)),
                accepted: crate::facts::AcceptedShapes::new(Arc::clone(&budget), crate::facts::accepted_weight),
                facts: Facts::new(budget),
                document: DocumentId::fresh(),
                strict,
            }),
        }
    }

    /// Whether the document is a strict Open XML document (found at open): only its layout
    /// reads it; every tool but `render` refuses it, and no change applies to it.
    #[must_use]
    pub fn strict(&self) -> bool {
        self.shared.strict
    }

    /// The package, as every reader reads it.
    #[must_use]
    pub fn package(&self) -> &WordPackage {
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

    /// The document's identity, the same in every version of it.
    #[must_use]
    pub fn document(&self) -> DocumentId {
        self.shared.document
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
        let mut slot = self.reading.lock().unwrap_or_else(PoisonError::into_inner);
        let made = match &*slot {
            Reading::Ready(made) => return Ok(Arc::clone(made)),
            Reading::Pending(before) => Arc::new(Docx::open_after(&self.package, before)?),
        };
        *slot = Reading::Ready(Arc::clone(&made));
        Ok(made)
    }

    /// What this version changed against `before` (any other version of the document): its
    /// stories' blocks by key, from the two versions' indexes ([`VersionDelta::between`]).
    /// Where either version cannot be read, every part that differs, none read as a story.
    #[must_use]
    pub fn delta_from(&self, before: &WordDoc) -> VersionDelta {
        let (Ok(old), Ok(new)) = (before.docx(), self.docx()) else {
            return VersionDelta {
                span: None,
                parts: PartsDelta::between(before.package(), self.package()),
                stories: Vec::new(),
            };
        };
        // the document's stories (its readings', which key its blocks), interned under the
        // lock; the comparison on a copy of the table
        let mut stories = {
            let mut table = new.stories();
            for part in old.parts.iter().chain(&new.parts) {
                let _ = table.intern(&part.story);
            }
            table.clone()
        };
        VersionDelta::between(&old, &new, &mut stories)
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

    /// `patch`, a later splice of this version's transaction in `part`, naming what the
    /// transaction's earlier splices inserted as they are here ([`redrawn::Redrawn::patch`]);
    /// `None` where it names none.
    fn redrawn_patch(&self, part: &str, patch: &Patch, draws: &[Draw]) -> Option<Patch> {
        if self.redrawn.is_empty() {
            return None;
        }
        let tree = self.package.tree(part).ok()??;
        self.redrawn.patch(patch, draws, &tree)
    }

    /// The next version: `package` and `ids`, its reading made from this one's.
    fn succeed(&self, package: Package, ids: Ids) -> WordDoc {
        let reading = self.reading.lock().unwrap_or_else(PoisonError::into_inner).handed_on();
        let tally = self.tally.lock().unwrap_or_else(PoisonError::into_inner).handed_on();
        WordDoc {
            package: WordPackage::edited(package),
            ids,
            reading: Arc::new(Mutex::new(Reading::Pending(reading))),
            tally: Arc::new(Mutex::new(Lazy::Pending(tally))),
            shared: Arc::clone(&self.shared),
            drawn: self.drawn.clone(),
            redrawn: self.redrawn.clone(),
            spaced: self.spaced,
            as_given: self.as_given,
            begun: self.begun.clone(),
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
                DrawFrom::Renewed { part, para_id, .. } if !self.para_id_written(part, para_id) => {
                    // kept: the value the element it replaces holds here (as the
                    // transaction's earlier splices left it), not the one it was prepared
                    // with -- a value drawn on a version where the part was written
                    let kept = match patch.removed().get(draw.node as usize) {
                        Some(ooxml_xml::Node::Element(old)) => attribute_below(old, &draw.at, &draw.attribute),
                        _ => None,
                    };
                    match kept {
                        Some(kept) => kept.to_owned(),
                        None => continue,
                    }
                }
                DrawFrom::Renewed { seed, .. } => {
                    let (values, change) = ids.draw(seed, 1, &self.shared.sequences);
                    ids.apply(&change);
                    added.values.extend(change.values);
                    added.cursors.extend(change.cursors);
                    values.into_iter().next().ok_or_else(stale)?
                }
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
        let mut package = self.package.package().clone();
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

    /// Whether the id of the block of `part` whose paraId is `para_id` has been written (in
    /// this version). A textId renews against the version last written: an id given at open
    /// and never written has been seen by no reader of the file, so it stays fresh for every
    /// write of the transaction that began with its part unwritten (a text edit keeps it
    /// whole, the second edit of the block in that transaction too); the next transaction
    /// renews it. It is a property of the version, which no key names: an edit decides by it
    /// where its change is applied ([`DrawFrom::Renewed`]), or reads the whole document.
    #[must_use]
    pub fn para_id_written(&self, part: &str, para_id: &str) -> bool {
        let base = self.begun.as_ref().unwrap_or(&self.package);
        if !matches!(base.part(part), Some(ooxml_opc::PartVersion::Unwritten(_))) {
            return true;
        }
        let Some(now) = ooxml_word_model::ids::hex_value(para_id) else {
            return true;
        };
        // a value the open gave is never one the saved part holds (draws skip those)
        base.saved_tree(part)
            .ok()
            .flatten()
            .is_some_and(|saved| ooxml_word_model::ids::used_long_hex(&[&**saved.root()]).contains(&now))
    }

    /// The package with `part` as another editor's version `from` has it
    /// ([`super::Change::Arrived`]): `patches` applied to the part here (or the part whole
    /// where `None`); saved as `from` holds it where the part here is `on`, the part the
    /// patches were worked out against; stamped where it lands by the rule of a part another
    /// writer put ([`ooxml_word_model::stamp::received`]), every part read with ids given at open written.
    pub(crate) fn arrived(
        &self,
        part: &str,
        patches: Option<&[ooxml_xml::Patch]>,
        from: &Package,
        on: Option<&ooxml_opc::PartVersion>,
    ) -> Result<(Package, Ids), ModelError> {
        let here = self.package.part(part);
        let before = self.package.tree(part).ok().flatten();
        let written = self.package.written();
        let (package, tree) = match (patches, &before) {
            (Some(patches), Some(before_tree)) => {
                let mut tree = before_tree.clone();
                for patch in patches {
                    tree = patch.apply(&tree)?;
                }
                let as_theirs = matches!((here, on), (Some(here), Some(on)) if here.same(on));
                let package = if as_theirs {
                    let theirs = from.tree(part)?;
                    if theirs.as_ref().is_some_and(|t| t.root().head() == tree.root().head()) {
                        written.with_part_as_in(part, from, Some(tree.clone()))?
                    } else {
                        // read with what ours declares in memory beyond theirs, saved as theirs
                        written
                            .with_part_as_in(part, from, None)?
                            .with_unwritten(part, tree.clone())?
                    }
                } else {
                    written.with_tree(part, tree.clone())?
                };
                (package, tree)
            }
            (Some(_), None) => return Err(ModelError::NoPart(part.to_owned())),
            (None, _) => {
                let package = written.with_part_as_in(part, from, None)?;
                let tree = match package.tree(part) {
                    Ok(Some(tree)) => tree,
                    // not XML: nothing to stamp
                    _ => return Ok((package, self.ids.clone())),
                };
                (package, tree)
            }
        };
        self.stamped_in(package, part, before, &tree)
    }

    /// The package with `part`'s root declaring what Word 2010's ids need
    /// ([`super::Change::Declare`], [`ooxml_word_model::stamp::ensured_head`] on the root
    /// here), repeats below cleaned away where it declares one anew; `None` when the root
    /// declares them already (or there is no such part).
    pub(crate) fn declared(
        &self,
        part: &str,
        drawings: bool,
        extra: &[(Arc<str>, Arc<str>)],
    ) -> Result<Option<Package>, ModelError> {
        let Some(tree) = self.package.tree(part)? else {
            return Ok(None);
        };
        let extra: Vec<(&str, &str)> = extra.iter().map(|(p, u)| (&**p, &**u)).collect();
        let root = tree.root();
        let Some((head, declared)) = ooxml_word_model::stamp::ensured_head(root, drawings, &extra) else {
            return Ok(None);
        };
        let mut new = Arc::new(ooxml_xml::Element::from_head(Arc::new(head), root.children().clone()));
        if declared {
            // lxml declares them with a cleanup of the whole part: declarations below that
            // repeat one in scope go
            new = ooxml_word_model::stamp::namespaces::cleaned(&new);
        }
        Ok(Some(self.package.with_tree(part, tree.with_root(new))?))
    }

    /// The package after a change of a root's head ([`super::Change::Declare`],
    /// [`super::Change::RootHead`]): the package as it is where it changes nothing here.
    pub(crate) fn headed(&self, change: &super::Change) -> Result<Package, ModelError> {
        let same = || self.package.package().clone();
        Ok(match change {
            super::Change::Declare { part, drawings, extra } => {
                self.declared(part, *drawings, extra)?.unwrap_or_else(same)
            }
            super::Change::RootHead { part, head } => self.root_merged(part, head)?,
            _ => same(),
        })
    }

    /// The package with `part`'s root head merged with `theirs` ([`super::Change::RootHead`]).
    pub(crate) fn root_merged(&self, part: &str, theirs: &ooxml_xml::Head) -> Result<Package, ModelError> {
        let tree = self
            .package
            .tree(part)?
            .ok_or_else(|| ModelError::NoPart(part.to_owned()))?;
        let root = tree.root();
        let ours = root.head();
        let mut namespaces = ours.namespaces.to_vec();
        for declared in theirs.namespaces.iter() {
            if !namespaces.iter().any(|d| d.prefix == declared.prefix) {
                namespaces.push(declared.clone());
            }
        }
        let ignorable = |head: &ooxml_xml::Head| -> Vec<String> {
            head.attributes
                .iter()
                .find(|a| a.name.is(Some(ooxml_word_model::ids::MC), "Ignorable"))
                .map(|a| a.value.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default()
        };
        let mut listed = ignorable(ours);
        for prefix in ignorable(theirs) {
            if !listed.contains(&prefix) {
                listed.push(prefix);
            }
        }
        let mut attributes = ours.attributes.to_vec();
        for attribute in theirs.attributes.iter() {
            match attributes.iter_mut().find(|a| a.name == attribute.name) {
                Some(a) => a.value = attribute.value.clone(),
                None => attributes.push(attribute.clone()),
            }
        }
        if let Some(a) = attributes
            .iter_mut()
            .find(|a| a.name.is(Some(ooxml_word_model::ids::MC), "Ignorable"))
        {
            a.value = Arc::from(listed.join(" "));
        }
        let head = ooxml_xml::Head {
            name: ours.name.clone(),
            namespaces: namespaces.into(),
            attributes: attributes.into(),
        };
        let new = ooxml_xml::Element::from_head(Arc::new(head), root.children().clone());
        Ok(self.package.with_tree(part, tree.with_root(Arc::new(new)))?)
    }

    /// `package`, which holds `tree` as `part`, with the part stamped where it is a story
    /// part whose blocks lack valid, unique ids ([`ooxml_word_model::stamp::received`]),
    /// `before` the part as this version had it, and the ids the stamps drew.
    fn stamped_in(
        &self,
        package: Package,
        part: &str,
        before: Option<Tree>,
        tree: &Tree,
    ) -> Result<(Package, Ids), ModelError> {
        let main = ooxml_word_model::parts::main_part(&package)?;
        let story = match &main {
            Some(main) if *main == ooxml_opc::name::normalize(part) => Some("body".to_owned()),
            Some(main) => ooxml_word_model::parts::story_parts(&package, main)?
                .iter()
                .any(|p| *p == ooxml_opc::name::normalize(part))
                .then(|| ooxml_word_model::ids::story_name(main, part)),
            None => None,
        };
        let Some(story) = story else {
            return Ok((package, self.ids.clone()));
        };
        // the values every other story part holds, and those the part holds itself
        let mut held = std::collections::HashSet::new();
        for name in ooxml_word_model::parts::parts(&package)? {
            if let Some(other) = package.tree(&name)? {
                held.extend(ooxml_word_model::ids::used_long_hex(&[&**other.root()]));
            }
        }
        let mut ids = self.ids.clone();
        let sequences = &self.shared.sequences;
        let mut draw = |seed: &str, count: usize| {
            let (values, change) = ids.draw(seed, count, sequences);
            ids.apply(&change);
            Ok::<_, std::convert::Infallible>(values)
        };
        let name = ooxml_opc::name::normalize(part);
        let found = ooxml_word_model::stamp::received(
            (&story, &name),
            before.as_ref().map(Tree::root),
            tree.root(),
            &|value| held.contains(&value),
            &mut draw,
        );
        let Ok(found) = found;
        match found {
            Some(stamp) => {
                // what the part kept, stamped as before, is the version before's own
                let stamped = ooxml_xml::shared_with(before, tree.with_root(stamp.root));
                Ok((package.with_unwritten(part, stamped)?, ids))
            }
            None => Ok((package, self.ids.clone())),
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
