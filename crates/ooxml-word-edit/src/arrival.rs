//! Owns: [`Arrival`], another editor's version of a Word document as a commit (co-authoring
//! refresh): the parts it changed against the version that editor's save was made from (its
//! base), and for each story part the changes as patches with the blocks they touch, by key.
//! Keys name every block a patch removes, inserts or changes, with the blocks it lies in (the
//! paragraph, the row and the table), so an edit of one row conflicts with another editor's
//! change inside it. The ids are the other editor's: the paraIds and textIds of the blocks it
//! wrote, added to the ledger so no draw here gives them again; a block left without a valid,
//! unique id where the change lands is stamped there ([`crate::Change::Arrived`]).
//!
//! [`Arrival::prepare`] makes the change on the head it lands on: where a part there is still
//! the base's, its patches as they are (and the part saves as the other editor wrote it);
//! where it changed since, the patches moved past what changed. A patch that cannot be moved
//! is [`Unplaced`]: the part is then taken whole ([`Arrival::widen`]), every step of this
//! session's that touched it handed back first.
//!
//! Must not: decide which steps are handed back (the store does, by [`Arrival::touches`]), or
//! refuse a change because of this session's steps: the other editor's version is the truth.

use crate::doc::{Change, Key, Setup, WordDoc};
use crate::pool::IdsChange;
use crate::{EditError, Editor};
use ooxml_opc::{Package, PartName};
use ooxml_store::{Caller, HandedBack, KeySet, Prepared, Seq, Snapshot, StoreError, Txn};
use ooxml_word_model::ids::{BlockKey, PartIds, StoryDelta, StoryIx};
use ooxml_xml::{Element, Patch, Tree};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::Arc;

/// Another editor's version of the document, as the change from its base.
#[derive(Debug, Clone)]
pub struct Arrival {
    from: Package,
    parts: Vec<ArrivedPart>,
}

#[derive(Debug, Clone)]
struct ArrivedPart {
    name: PartName,
    how: How,
    /// The keys it writes (and reads: it replaces what is there).
    keys: BTreeSet<Key>,
    /// Those a step here can have touched: all but the blocks only theirs has (a step here
    /// that names one drew the same id, which is stamped anew where it lands).
    touched: BTreeSet<Key>,
    /// The blocks only theirs has: kept out of what it writes where the part here holds
    /// their id already (stamped anew where it lands, so the key would name ours).
    came: BTreeSet<Key>,
    /// Their root's head where it differs from the base's (merged where it lands:
    /// [`Change::RootHead`]).
    root: Option<Arc<ooxml_xml::Head>>,
}

#[derive(Debug, Clone)]
enum How {
    /// A story part both versions have: the patches from the base's part to theirs.
    Patched(Vec<Patch>),
    /// The part whole as theirs is (a part that is not a story, a new one, one whose
    /// patches could not be placed).
    Whole,
    /// Gone from theirs.
    Removed,
    /// The people part, theirs listing these authors more than the base's (each listed
    /// where it lands, [`Change::People`]): a union by author.
    People(Vec<Arc<str>>),
    /// A part the base did not have: theirs, and one of ours added since under the same
    /// name moved to a name of its own ([`Change::Moved`]).
    Added,
    /// A relationships part: the union by id of theirs and what was added here since the
    /// base, an id of ours theirs gives another entry renamed ([`Change::Relinked`]).
    Rels(Option<Tree>, Tree),
    /// The content types: the union by part name and extension ([`Change::Typed`]).
    Types(Option<Tree>, Tree),
}

/// A part whose patches could not be moved past what changed in it since the base: take it
/// whole ([`Arrival::widen`]) and receive again.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("another editor's change of {0} could not be placed past the changes made here since")]
pub struct Unplaced(pub String);

/// Why [`Arrival::prepare`] did not make the change.
#[derive(Debug, thiserror::Error)]
pub enum ArrivalError {
    /// A part's patches could not be placed.
    #[error(transparent)]
    Unplaced(#[from] Unplaced),
    /// The document or the store refused.
    #[error(transparent)]
    Edit(#[from] EditError),
}

impl From<ooxml_store::StoreError<Key, crate::ModelError>> for ArrivalError {
    fn from(error: ooxml_store::StoreError<Key, crate::ModelError>) -> Self {
        ArrivalError::Edit(error.into())
    }
}

impl Arrival {
    /// The change from `base` (the version the other editor's save was made from) to
    /// `from` (that editor's version, opened from its bytes).
    ///
    /// # Errors
    /// When a story part of either version is not XML.
    pub fn plan(base: &WordDoc, from: Package) -> Result<Arrival, EditError> {
        let docx = base.docx()?;
        let mut parts = Vec::new();
        for name in from.changed_parts(base.package()) {
            let name = PartName::normalize(&name);
            if !from.has_part(&name) {
                parts.push(ArrivedPart::whole(name, How::Removed));
                continue;
            }
            if let Some(merged) = bookkeeping(base, &from, &name)? {
                parts.push(ArrivedPart {
                    keys: BTreeSet::from([Key::Part(name.clone())]),
                    touched: BTreeSet::new(),
                    came: BTreeSet::new(),
                    root: None,
                    name,
                    how: merged,
                });
                continue;
            }
            if let Some(authors) = added_people(base, &docx.main, &from, &name)? {
                parts.push(ArrivedPart {
                    keys: BTreeSet::from([Key::People]),
                    touched: BTreeSet::new(),
                    came: BTreeSet::new(),
                    root: None,
                    name,
                    how: How::People(authors),
                });
                continue;
            }
            let story = docx.parts.iter().find(|story| story.part.as_str() == name.as_str());
            let theirs = story.map(|_| from.tree(&name)).transpose()?.flatten();
            let planned = match (story, theirs) {
                (Some(story), Some(theirs)) => patched(&story.story, &name, &story.tree, &story.ids, &theirs),
                _ => None,
            };
            parts.push(planned.unwrap_or_else(|| ArrivedPart::whole(name, How::Whole)));
        }
        Ok(Arrival { from, parts })
    }

    /// Whether the other editor's version changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// The parts it changes, in its version's order (removed ones last).
    #[must_use]
    pub fn parts(&self) -> Vec<&str> {
        self.parts.iter().map(|p| p.name.as_str()).collect()
    }

    /// The blocks and parts it changes, as keys.
    #[must_use]
    pub fn keys(&self) -> KeySet<Key> {
        KeySet::of(self.parts.iter().flat_map(|p| p.keys.iter().cloned()))
    }

    /// The ids of the blocks it changes (`p:1A2B3C4D`, `t:…`, `tr:…`), in part order.
    #[must_use]
    pub fn blocks(&self) -> Vec<String> {
        let mut out = Vec::new();
        for part in &self.parts {
            for key in &part.keys {
                if let Key::Block { id, .. } = key {
                    out.push(id.to_string());
                }
            }
        }
        out
    }

    /// Whether a step that read `reads` and wrote `writes` (`merges` of them only as merges,
    /// made where its change landed) touched what it changes: a block it changes, the whole
    /// of a part it changes, or anything of a part it takes whole. A key written only as a
    /// merge (a person listed, a declaration made) touches a part it merges too.
    #[must_use]
    pub fn touches(&self, reads: &KeySet<Key>, writes: &KeySet<Key>, merges: &KeySet<Key>) -> bool {
        // (a part whose content is theirs as it was, saved otherwise, touches nothing)
        if self.parts.iter().all(|part| part.touched.is_empty()) {
            return false;
        }
        let (Some(reads), Some(all)) = (reads.keys(), writes.keys()) else {
            return true;
        };
        let unmerged = writes.without(merges);
        let writes = unmerged.keys().unwrap_or(all);
        self.parts.iter().any(|part| {
            let mine = |key: &Key| part_of(key) == Some(&part.name);
            match part.how {
                How::Whole | How::Removed => reads.iter().chain(all).any(mine),
                // merged by id and name, ours renamed or moved where both name one
                How::Added | How::Rels(..) | How::Types(..) => false,
                How::People(_) => writes.iter().any(mine),
                How::Patched(_) => {
                    // one of its blocks read or written, the part (or its root) written whole,
                    // or what its paragraphs show of their styles read (derived: a writer of
                    // it is none)
                    let whole = |key: &Key| mine(key) && matches!(key, Key::Part(_) | Key::Root(_));
                    reads.iter().chain(writes).any(|key| part.touched.contains(key))
                        || writes.iter().any(whole)
                        || reads
                            .iter()
                            .any(|key| matches!(key, Key::StyleUse(p) if *p == part.name))
                }
            }
        })
    }

    /// Take the part `part` whole: its patches could not be placed ([`Unplaced`]).
    pub fn widen(&mut self, part: &str) {
        for arrived in self.parts.iter_mut().filter(|p| *p.name == *part) {
            arrived.how = How::Whole;
            arrived.keys.insert(Key::Part(arrived.name.clone()));
            arrived.touched.insert(Key::Part(arrived.name.clone()));
            arrived.root = None;
        }
    }

    /// The part whole as `from` holds it, writing `keys`.
    fn whole(&self, name: Arc<str>, keys: Arc<[Key]>) -> Change {
        Change::Arrived {
            part: name,
            patches: None,
            keys,
            from: self.from.clone(),
            on: None,
        }
    }

    /// A part the base did not have, theirs whole; one of ours added since under the same
    /// name (other bytes) moved first to a name of its own. Keyed by nothing: a part of ours
    /// made meanwhile under that name is a place both changes take, so that transaction is
    /// prepared again and names its part anew, not refused.
    fn added(&self, tx: &mut Txn<'_, WordDoc>, name: Arc<str>) -> Result<Change, ArrivalError> {
        let ours = tx.doc().package().part(&name).is_some();
        if ours && !same_bytes(tx.doc().package(), &self.from, &name)? {
            let to = crate::merging::fresh_name(tx.doc().package(), &self.from, &name);
            let rels = crate::merging::targeting(tx.doc().package(), &name).map_err(EditError::from)?;
            tx.write(Change::Moved {
                from: Arc::clone(&name),
                to: Arc::from(to.as_str()),
                rels: rels.iter().map(|r| Arc::from(r.as_str())).collect(),
            })?;
        }
        Ok(self.whole(name, Arc::from([])))
    }

    /// A story part's patches as they land on `tx`'s head (as they are where the part there
    /// is still `on`, the base's: `same`), their root's head merged after them.
    fn patched_on(
        &self,
        tx: &mut Txn<'_, WordDoc>,
        base: &WordDoc,
        part: &ArrivedPart,
        (name, keys): (Arc<str>, Arc<[Key]>),
        (patches, on, same): (&[Patch], Option<ooxml_opc::PartVersion>, bool),
    ) -> Result<Change, ArrivalError> {
        let placed = if same {
            patches.to_vec()
        } else {
            placed_past(patches, &base.tree(&name)?, &tx.doc().tree(&name)?)
                .ok_or_else(|| Unplaced(name.to_string()))?
        };
        let Some(head) = &part.root else {
            return Ok(Change::Arrived {
                part: name,
                patches: Some(placed.into()),
                keys,
                from: self.from.clone(),
                on: on.filter(|_| same),
            });
        };
        // their root's head merged where it lands, after their children
        tx.write(Change::Arrived {
            part: Arc::clone(&name),
            patches: Some(placed.into()),
            keys,
            from: self.from.clone(),
            on: None,
        })?;
        Ok(Change::RootHead {
            part: name,
            head: Arc::clone(head),
        })
    }

    /// Make the change in `tx`, on its head, `base` the version it was made from: each part
    /// as theirs, and the other editor's id values added to the ledger.
    ///
    /// # Errors
    /// [`Unplaced`] for a part whose patches cannot be moved past what changed in it here
    /// since `base`; what the store or the document refused.
    pub fn prepare(&self, tx: &mut Txn<'_, WordDoc>, base: &WordDoc) -> Result<(), ArrivalError> {
        let mut trees = Vec::new();
        // the bookkeeping last, merged over the parts as they land
        let last = |p: &&ArrivedPart| matches!(p.how, How::Rels(..) | How::Types(..));
        let ordered = self
            .parts
            .iter()
            .filter(|p| !last(p))
            .chain(self.parts.iter().filter(last));
        for part in ordered {
            let keys = part.written_on(tx.doc())?;
            for key in &keys {
                tx.read(key.clone());
            }
            let name: Arc<str> = Arc::clone(part.name.as_arc());
            let keys: Arc<[Key]> = keys.into_iter().collect();
            let on = base.package().part(&name).cloned();
            let here = tx.doc().package().part(&name);
            let same = matches!((&on, here), (Some(on), Some(here)) if on.same(here));
            let change = match &part.how {
                How::People(authors) if !same => {
                    let mut sink = crate::sink::EditSink::new(tx, crate::policy::RevisionPolicy::Untracked);
                    for author in authors {
                        sink.people(Some(author), None)?;
                    }
                    continue;
                }
                How::Removed if !tx.doc().package().has_part(&name) => continue,
                How::Removed => Change::Remove { part: name },
                How::Rels(base, theirs) => Change::Relinked {
                    part: name,
                    base: base.clone(),
                    theirs: theirs.clone(),
                    from: self.from.clone(),
                    keys,
                },
                How::Types(base, theirs) => Change::Typed {
                    base: base.clone(),
                    theirs: theirs.clone(),
                },
                How::Added => self.added(tx, name)?,
                How::Patched(patches) => self.patched_on(tx, base, part, (name, keys), (patches, on, same))?,
                // the part whole as theirs (people with the part here as at the base)
                How::Whole | How::People(_) => self.whole(name, keys),
            };
            if !matches!(part.how, How::Removed | How::Rels(..) | How::Types(..)) {
                if let Ok(Some(tree)) = self.from.tree(&part.name) {
                    trees.push(tree);
                }
            }
            tx.write(change)?;
        }
        let trees: Vec<&Tree> = trees.iter().collect();
        if let Some(ledger) = ledger_of(tx.doc(), &trees) {
            tx.write(ledger)?;
        }
        Ok(())
    }
}

/// What another editor's version did here ([`Editor::receive`]).
#[derive(Debug)]
pub struct Arrived {
    /// The version it made (the head it found, where it changed nothing).
    pub version: Snapshot<WordDoc>,
    /// The version it was published after, where it changed something.
    pub previous: Option<Snapshot<WordDoc>>,
    /// The steps of this session's taken back first, newest first: each touched what the
    /// other editor changed (or was entangled with one that did).
    pub handed_back: Vec<HandedBack<WordDoc>>,
    /// The parts it changed.
    pub parts: Vec<String>,
    /// The blocks it changed, by id.
    pub blocks: Vec<String>,
}

impl Editor {
    /// Take in another editor's version of the document, `from` (opened from its bytes),
    /// made from version `base` of this one: committed as one step by `editor` (a foreign
    /// caller, [`Caller::foreign`]), after the steps since `base` that touch what it changed
    /// are taken back ([`ooxml_store::Store::receive`]).
    ///
    /// # Errors
    /// [`StoreError::NotKept`] when `base` is not kept with the steps after it; a story part
    /// that is not XML; a take-back the history refuses.
    pub fn receive(&self, editor: &str, base: Seq, from: Package) -> Result<Arrived, EditError> {
        let on = self.store().kept(base).ok_or(StoreError::NotKept { seq: base })?;
        let planned = Arrival::plan(on.doc(), from)?;
        if planned.is_empty() {
            // their version is this one's base as it is: nothing comes in, nothing is handed back
            return Ok(Arrived {
                version: self.store().snapshot(),
                previous: None,
                handed_back: Vec::new(),
                parts: Vec::new(),
                blocks: Vec::new(),
            });
        }
        let arrival = RefCell::new(planned);
        let received = self.store().receive(
            &Caller::foreign(editor),
            base,
            &Setup::default(),
            |step| arrival.borrow().touches(&step.reads, &step.writes, &step.merges),
            |tx, on| {
                let prepared = arrival.borrow().prepare(tx, on.doc());
                match prepared {
                    Ok(()) => Ok(Prepared::Made),
                    Err(ArrivalError::Unplaced(Unplaced(part))) => {
                        arrival.borrow_mut().widen(&part);
                        Ok(Prepared::Widened)
                    }
                    Err(ArrivalError::Edit(error)) => Err(error),
                }
            },
        )?;
        let arrival = arrival.into_inner();
        let committed = received.committed;
        Ok(Arrived {
            previous: committed.previous.filter(|_| committed.changed),
            version: committed.version,
            handed_back: received.handed_back,
            parts: arrival.parts().into_iter().map(str::to_owned).collect(),
            blocks: arrival.blocks(),
        })
    }
}

/// What `trees`, put in place by another writer, add to `doc`'s ledger: the id values they
/// hold that it lacks, and the drawing numbers, so that none is given again here.
#[must_use]
pub fn ledger_of(doc: &WordDoc, trees: &[&Tree]) -> Option<Change> {
    let roots: Vec<&ooxml_xml::Element> = trees.iter().map(|t| &**t.root()).collect();
    let mut values: Vec<u32> = ooxml_word_model::ids::used_long_hex(&roots)
        .into_iter()
        .filter(|v| !doc.ids().contains(*v))
        .collect();
    values.sort_unstable();
    // a drawing the parts bring is one the document had (a copy keeps no number it had),
    // and raises the highest number given (never given again)
    let mut drawings: Vec<u64> = ooxml_word_model::ids::drawing_numbers(&roots)
        .into_iter()
        .filter(|n| !doc.ids().had_drawing(*n))
        .collect();
    drawings.sort_unstable();
    let highest = drawings
        .last()
        .copied()
        .filter(|n| *n > doc.ids().highest_drawing())
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

impl ArrivedPart {
    /// `name` taken whole (or removed): it writes and touches the part.
    fn whole(name: PartName, how: How) -> ArrivedPart {
        let keys = BTreeSet::from([Key::Part(name.clone())]);
        ArrivedPart {
            touched: keys.clone(),
            keys,
            came: BTreeSet::new(),
            root: None,
            name,
            how,
        }
    }

    /// The keys it writes where it lands on `doc`: its keys but a block only theirs has
    /// whose id the part there holds already (stamped anew where it lands).
    fn written_on(&self, doc: &WordDoc) -> Result<Vec<Key>, EditError> {
        if self.came.is_empty() {
            return Ok(self.keys.iter().cloned().collect());
        }
        let docx = doc.docx()?;
        let story = docx
            .parts
            .iter()
            .find(|story| story.part.as_str() == self.name.as_str());
        Ok(self
            .keys
            .iter()
            .filter(|key| match key {
                Key::Block { id, .. } if self.came.contains(key) => {
                    story.is_none_or(|story| story.ids.get(id.as_ref()).is_none())
                }
                _ => true,
            })
            .cloned()
            .collect())
    }
}

/// How `name`, a part `from` changed against `base`, is merged where it is one of the
/// package's bookkeeping parts: a relationships part, the content types, or a part the base
/// did not have; `None` for any other.
fn bookkeeping(base: &WordDoc, from: &Package, name: &PartName) -> Result<Option<How>, EditError> {
    let theirs = || from.tree(name);
    if name.as_str().ends_with(".rels") {
        return Ok(theirs()?.map(|t| How::Rels(base.package().tree(name).ok().flatten(), t)));
    }
    if name.as_str() == ooxml_opc::CONTENT_TYPES_PART {
        return Ok(theirs()?.map(|t| How::Types(base.package().tree(name).ok().flatten(), t)));
    }
    Ok((!base.package().has_part(name)).then_some(How::Added))
}

/// Whether `ours` and `theirs` hold the part `name` with the same bytes.
fn same_bytes(ours: &Package, theirs: &Package, name: &str) -> Result<bool, EditError> {
    Ok(ours.read(name)? == theirs.read(name)?)
}

/// The authors `from`'s people part lists beyond `base`'s, where `name` is the base's people
/// part and theirs only adds people (every person the base lists kept as it is); `None`
/// otherwise (the part is taken as any other).
fn added_people(
    base: &WordDoc,
    main: &str,
    from: &Package,
    name: &PartName,
) -> Result<Option<Vec<Arc<str>>>, EditError> {
    if crate::people::people_part(base.package(), main)?.as_deref() != Some(name.as_str()) {
        return Ok(None);
    }
    let (Some(ours), Some(theirs)) = (base.package().tree(name)?, from.tree(name)?) else {
        return Ok(None);
    };
    let people = |root: &ooxml_xml::Element| -> Vec<(Option<String>, Arc<ooxml_xml::Element>)> {
        root.child_elements()
            .map(|e| {
                (
                    e.attribute(Some(crate::names::W15), "author").map(str::to_owned),
                    Arc::clone(e),
                )
            })
            .collect()
    };
    if ours.root().head() != theirs.root().head() {
        return Ok(None);
    }
    let (ours, theirs) = (people(ours.root()), people(theirs.root()));
    let kept = ours
        .iter()
        .all(|(author, element)| author.is_some() && theirs.iter().any(|(a, e)| a == author && e == element));
    if !kept {
        return Ok(None);
    }
    let added: Vec<Arc<str>> = theirs
        .iter()
        .filter(|(author, _)| !ours.iter().any(|(a, _)| a == author))
        .filter_map(|(author, _)| author.as_deref().map(Arc::from))
        .collect();
    Ok(Some(added))
}

/// The part a key is of.
fn part_of(key: &Key) -> Option<&PartName> {
    match key {
        Key::Block { part, .. } | Key::Part(part) | Key::Root(part) | Key::StyleUse(part) => Some(part),
        Key::Seq(_) | Key::Annotations | Key::People => None,
    }
}

/// A story part both versions have, as patches from ours (`tree`, indexed as `ids`) to
/// `theirs`, keyed by the blocks they touch; `None` where the part is better taken whole
/// (its block container moved, or its root changed).
fn patched(story: &str, name: &PartName, tree: &Tree, ids: &PartIds, theirs: &Tree) -> Option<ArrivedPart> {
    let new = PartIds::new(story, theirs.root());
    let StoryDelta::Blocks(delta) = ids.delta(tree.root(), &new, theirs.root(), StoryIx::BODY) else {
        return None;
    };
    // their root's head (declarations, attributes) merged where it lands; the children
    // patched as the base's root holds them
    let (root, children) = if theirs.root().head() == tree.root().head() {
        (None, theirs.clone())
    } else if theirs.root().name() == tree.root().name() {
        let base_head = Arc::clone(tree.root().head());
        let adjusted = Element::from_head(Arc::clone(&base_head), theirs.root().children().clone());
        // (a head of theirs with nothing ours lacks -- ours declaring in memory what theirs
        // leaves out, say -- merges to ours: nothing to merge)
        let more = !head_within(theirs.root().head(), &base_head);
        (
            more.then(|| Arc::clone(theirs.root().head())),
            theirs.with_root(Arc::new(adjusted)),
        )
    } else {
        return None;
    };
    let patches = ooxml_xml::diff(tree, &children);
    if patches.iter().any(|p| p.parent().0.is_empty()) {
        return None;
    }
    let (mut keys, mut touched) = (BTreeSet::new(), BTreeSet::new());
    // the block, and every block it lies in (blocks the base has); the block itself among
    // those a step here can have touched unless only theirs has it (`came`)
    let mut key = |index: &PartIds, block: BlockKey, came: bool| {
        if let Some(place) = index.place_of(block) {
            for len in 1..place.len() {
                let within = place.get(..len).unwrap_or_default();
                if let Some(id) = index.key_at(StoryIx::BODY, within).and_then(|k| index.id_of_key(k)) {
                    keys.insert(Key::block(name.clone(), id.clone()));
                    touched.insert(Key::block(name.clone(), id));
                }
            }
        }
        if let Some(id) = index.id_of_key(block) {
            keys.insert(Key::block(name.clone(), id.clone()));
            if !came {
                touched.insert(Key::block(name.clone(), id));
            }
        }
    };
    let old_blocks = delta
        .changed
        .iter()
        .copied()
        .chain(delta.gone.iter().map(|(k, _)| *k))
        .chain(delta.renamed.iter().map(|(k, _)| *k))
        .chain(delta.moved.iter().map(|m| m.key))
        .chain(delta.wrapped.iter().copied())
        .chain(delta.stamp_only.iter().copied());
    for block in old_blocks {
        key(ids, block, false);
    }
    for block in delta.changed.iter().copied() {
        key(&new, block, false);
    }
    let came = delta
        .came
        .iter()
        .map(|(k, _)| *k)
        .chain(delta.renamed.iter().map(|(_, k)| *k));
    for block in came {
        key(&new, block, true);
    }
    if delta.outside {
        keys.insert(Key::Part(name.clone()));
        touched.insert(Key::Part(name.clone()));
    }
    if root.is_some() {
        keys.insert(Key::Root(name.clone()));
        touched.insert(Key::Root(name.clone()));
    }
    let came = keys.difference(&touched).cloned().collect();
    Some(ArrivedPart {
        name: name.clone(),
        how: How::Patched(patches),
        keys,
        touched,
        came,
        root,
    })
}

/// Whether `theirs` declares and sets nothing `ours` lacks: each namespace declared there,
/// each attribute the same, `mc:Ignorable` listing no prefix ours does not.
fn head_within(theirs: &ooxml_xml::Head, ours: &ooxml_xml::Head) -> bool {
    let ignorable = |head: &ooxml_xml::Head| -> Vec<String> {
        head.attributes
            .iter()
            .find(|a| a.name.is(Some(ooxml_word_model::ids::MC), "Ignorable"))
            .map(|a| a.value.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default()
    };
    let listed = ignorable(ours);
    theirs.namespaces.iter().all(|d| ours.namespaces.contains(d))
        && theirs
            .attributes
            .iter()
            .all(|a| a.name.is(Some(ooxml_word_model::ids::MC), "Ignorable") || ours.attributes.contains(a))
        && ignorable(theirs).iter().all(|p| listed.contains(p))
}

/// `patches`, made on `base`, moved past what changed from `base` to `head` (one part's
/// trees), as a commit's changes are moved past those published since: each patch past each
/// of theirs, theirs carried past the patches before it. `None` when one cannot be moved.
fn placed_past(patches: &[Patch], base: &Option<Tree>, head: &Option<Tree>) -> Option<Vec<Patch>> {
    let (Some(base), Some(head)) = (base, head) else {
        return None;
    };
    let mut placed = patches.to_vec();
    let last = placed.len().saturating_sub(1);
    // a root's head changed since (a declaration made here) moves no child: what changed
    // below it is what the patches move past
    let head = if head.root().head() == base.root().head() || head.root().name() != base.root().name() {
        head.clone()
    } else {
        let children = Element::from_head(Arc::clone(base.root().head()), head.root().children().clone());
        head.with_root(Arc::new(children))
    };
    for mut past in ooxml_xml::diff(base, &head) {
        for (i, patch) in placed.iter_mut().enumerate() {
            let moved = patch.rebase(&past).ok()?;
            if i < last {
                past = past.rebase_first(patch).ok()?;
            }
            *patch = moved;
        }
    }
    Some(placed)
}
