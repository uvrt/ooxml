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
use ooxml_xml::{Patch, Tree};
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
                let keys = BTreeSet::from([Key::Part(name.clone())]);
                parts.push(ArrivedPart {
                    name,
                    how: How::Removed,
                    touched: keys.clone(),
                    keys,
                });
                continue;
            }
            let story = docx.parts.iter().find(|story| story.part.as_str() == name.as_str());
            let theirs = story.map(|_| from.tree(&name)).transpose()?.flatten();
            let planned = match (story, theirs) {
                (Some(story), Some(theirs)) => patched(&story.story, &name, &story.tree, &story.ids, &theirs),
                _ => None,
            };
            parts.push(planned.unwrap_or_else(|| ArrivedPart {
                keys: BTreeSet::from([Key::Part(name.clone())]),
                touched: BTreeSet::from([Key::Part(name.clone())]),
                name,
                how: How::Whole,
            }));
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

    /// Whether a step that read `reads` and wrote `writes` touched what it changes: a block
    /// it changes, the whole of a part it changes, or anything of a part it takes whole.
    #[must_use]
    pub fn touches(&self, reads: &KeySet<Key>, writes: &KeySet<Key>) -> bool {
        let (Some(reads), Some(writes)) = (reads.keys(), writes.keys()) else {
            return true;
        };
        self.parts.iter().any(|part| {
            let mine = |key: &Key| part_of(key) == Some(&part.name);
            match part.how {
                How::Whole | How::Removed => reads.iter().chain(writes).any(mine),
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
        }
    }

    /// Make the change in `tx`, on its head, `base` the version it was made from: each part
    /// as theirs, and the other editor's id values added to the ledger.
    ///
    /// # Errors
    /// [`Unplaced`] for a part whose patches cannot be moved past what changed in it here
    /// since `base`; what the store or the document refused.
    pub fn prepare(&self, tx: &mut Txn<'_, WordDoc>, base: &WordDoc) -> Result<(), ArrivalError> {
        let mut trees = Vec::new();
        for part in &self.parts {
            for key in &part.keys {
                tx.read(key.clone());
            }
            let name: Arc<str> = Arc::clone(part.name.as_arc());
            let keys: Arc<[Key]> = part.keys.iter().cloned().collect();
            let change = match &part.how {
                How::Removed if !tx.doc().package().has_part(&name) => continue,
                How::Removed => Change::Remove { part: name },
                How::Whole => Change::Arrived {
                    part: name,
                    patches: None,
                    keys,
                    from: self.from.clone(),
                    on: None,
                },
                How::Patched(patches) => {
                    let on = base.package().part(&name).cloned();
                    let here = tx.doc().package().part(&name);
                    let same = matches!((&on, here), (Some(on), Some(here)) if on.same(here));
                    let placed = if same {
                        patches.clone()
                    } else {
                        placed_past(patches, &base.tree(&name)?, &tx.doc().tree(&name)?)
                            .ok_or_else(|| Unplaced(name.to_string()))?
                    };
                    Change::Arrived {
                        part: name,
                        patches: Some(placed.into()),
                        keys,
                        from: self.from.clone(),
                        on: on.filter(|_| same),
                    }
                }
            };
            if !matches!(part.how, How::Removed) {
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
        let arrival = RefCell::new(Arrival::plan(on.doc(), from)?);
        let received = self.store().receive(
            &Caller::foreign(editor),
            base,
            &Setup::default(),
            |step| arrival.borrow().touches(&step.reads, &step.writes),
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
    let patches = ooxml_xml::diff(tree, theirs);
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
    Some(ArrivedPart {
        name: name.clone(),
        how: How::Patched(patches),
        keys,
        touched,
    })
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
    for mut past in ooxml_xml::diff(base, head) {
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
