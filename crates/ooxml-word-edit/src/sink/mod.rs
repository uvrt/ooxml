//! Owns: [`EditSink`], the one seam through which every Word edit writes (design section
//! 4): an element replaced by its edited copy, a root's head changed, a part put in place,
//! ids drawn from the ledger, annotation ids allocated, renames recorded -- each one a
//! primitive of the transaction's change set with the keys it reads and writes; and the
//! text primitive the [`RevisionPolicy`] turns into untracked or tracked markup
//! ([`EditSink::set_paragraph_text`]).
//!
//! Must not: be bypassed (no edit touches a tree any other way), commit (the store does),
//! or decide what an edit means.

mod bytes;
mod heads;
mod kept;
mod media;
mod numbering;
mod stand_ins;
mod stretch;
mod written;

pub(crate) use heads::{annotation_draws, ensured_head};

use crate::doc::{Change, Draw, Key, WordDoc};
use crate::policy::{RevisionPolicy, Stamp};
use crate::props::PropertyTracker;
use crate::text::write;
use crate::track;
use crate::work::Work;
use crate::{EditError, Result};
use ooxml_store::Txn;
use ooxml_word_model::docx::Docx;
use ooxml_xml::{Element, HeadPatch, Node, NodePath, Patch};
use std::sync::Arc;

/// What a paragraph's text edit did.
#[derive(Debug, Default)]
pub struct TextWrite {
    /// Whether the text changed.
    pub changed: bool,
    /// Relationship ids of what the edit removed outright (to release).
    pub released: Vec<String>,
    /// Whether a revision record carries `w16du:dateUtc`: the part's root is to declare it
    /// ([`EditSink::ensure_w14`] with `w16du`, once the paragraph is in place).
    pub dated: bool,
    /// The revision ids the write gave (tracked): stand-ins, drawn again where the change
    /// is applied ([`crate::DrawFrom::Annotation`]).
    pub annotations: Option<Numbered>,
}

/// Revision ids a write gave, `from..to`, of an edit numbering from `base`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Numbered {
    /// The edit's first id.
    pub base: i64,
    /// The first this write gave.
    pub from: i64,
    /// One above the last this write gave.
    pub to: i64,
}

/// How a replaced stretch's new text takes its formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Keep {
    /// All of it the first replaced character's (docx-agent's `keep="first"`).
    #[default]
    First,
    /// Each surviving character its own, new ones the character they replace
    /// (`keep="characters"`; tracked, the changed words only are revisions).
    Characters,
}

/// The sink a command writes through, over one transaction.
pub struct EditSink<'t, 's> {
    tx: &'t mut Txn<'s, WordDoc>,
    policy: RevisionPolicy,
    /// How this edit numbers its revision records ([`numbering`]).
    numbered: numbering::Numbering,
    /// The text the edit wrote, by place ([`crate::spacing`]).
    written: crate::spacing::Written,
}

impl<'t, 's> EditSink<'t, 's> {
    /// A sink over `tx`, writing by `policy`.
    pub fn new(tx: &'t mut Txn<'s, WordDoc>, policy: RevisionPolicy) -> Self {
        EditSink {
            tx,
            policy,
            numbered: numbering::Numbering::default(),
            written: crate::spacing::Written::default(),
        }
    }

    /// The working version: the snapshot with this transaction's changes so far.
    #[must_use]
    pub fn doc(&self) -> &WordDoc {
        self.tx.doc()
    }

    /// The snapshot the transaction started from.
    #[must_use]
    pub fn base(&self) -> &WordDoc {
        self.tx.base().doc()
    }

    /// The working version as docx-agent reads it.
    ///
    /// # Errors
    /// When a story part is not XML.
    pub fn docx(&self) -> Result<Arc<Docx>> {
        self.tx.doc().docx()
    }

    /// The policy edits are written by.
    #[must_use]
    pub fn policy(&self) -> &RevisionPolicy {
        &self.policy
    }

    /// Run `write` with this sink writing by `policy` (a part of an edit docx-agent writes
    /// untracked whatever the mode: a table of contents' holder paragraph), then by its own
    /// policy again.
    ///
    /// # Errors
    /// `write`'s.
    pub(crate) fn with_policy<R>(
        &mut self,
        policy: RevisionPolicy,
        write: impl FnOnce(&mut Self) -> Result<R>,
    ) -> Result<R> {
        let own = std::mem::replace(&mut self.policy, policy);
        let out = write(self);
        self.policy = own;
        out
    }

    /// Record that the edit depends on the whole document (a find over every paragraph):
    /// it commits only when nothing came between its snapshot and its commit.
    pub fn read_everything(&mut self) {
        self.tx.read_everything();
    }

    /// Record that the edit depends on every key `predicate` names on the head
    /// ([`ooxml_store::Txn::read_where`]).
    pub(crate) fn read_where(&mut self, predicate: Arc<dyn ooxml_store::ReadPredicate<WordDoc>>) {
        self.tx.read_where(predicate);
    }

    /// Record that the edit depends on `key`.
    pub fn read(&mut self, key: Key) {
        self.tx.read(key);
    }

    fn write(&mut self, change: Change) -> Result<()> {
        let change = self.placed(change);
        if let Change::Splice { draws, .. } = &change {
            self.numbered.written(draws);
        }
        self.tx.write(change).map_err(EditError::from)
    }

    /// `count` new ids of `seed`: stand-ins, the values this version gives, drawn again
    /// where the change is applied ([`Change::Draw`]), so two transactions drawing from one
    /// seed do not conflict and each gets what the commit order gives. Written into the
    /// splices after it, they are drawn there too; anywhere else they are pinned
    /// ([`stand_ins`]).
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub fn draw(&mut self, seed: &str, count: usize) -> Result<Vec<String>> {
        if count == 0 {
            // nothing drawn: the sequence neither read nor written (no conflict with a
            // writer drawing from it)
            return Ok(Vec::new());
        }
        let (values, _) = self.tx.doc().ids().draw(seed, count, &self.tx.doc().shared().sequences);
        let seed: Arc<str> = Arc::from(seed);
        let stand_ins: Arc<[Arc<str>]> = values.iter().map(|v| Arc::from(v.to_uppercase())).collect();
        self.tx
            .write(Change::Draw {
                seed,
                stand_ins: Arc::clone(&stand_ins),
            })
            .map_err(EditError::from)?;
        Ok(values)
    }

    /// `count` new drawing numbers (`wp:docPr/@id`), past every one the document has and
    /// had (read and written: two transactions drawing them are ordered by their commits).
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub fn draw_drawings(&mut self, count: usize) -> Result<Vec<u64>> {
        if count == 0 {
            return Ok(Vec::new());
        }
        self.tx.read(Key::Seq(Arc::from(crate::pool::DRAWING_NUMBERS)));
        let (values, change) = self.tx.doc().ids().draw_drawings(count);
        self.write(Change::Ids(change))?;
        Ok(values)
    }

    /// Record renames (an old id and the id it names now).
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub fn rename(&mut self, renames: Vec<(String, String)>) -> Result<()> {
        if renames.is_empty() {
            return Ok(());
        }
        self.write(Change::Ids(crate::pool::IdsChange {
            aliases: renames,
            ..Default::default()
        }))
    }

    /// Forget the aliases of ids an edit removed (an old id naming a block that went).
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub(crate) fn forget_aliases(&mut self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        self.write(Change::Ids(crate::pool::IdsChange {
            forget: ids.to_vec(),
            ..Default::default()
        }))
    }

    /// One block replaced by one block (docx-agent's `_note_replaced`): a refusal naming
    /// `old`'s id says `new` replaced it. Anything else notes nothing.
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub(crate) fn note_replaced(&mut self, old: &[String], new: &[String]) -> Result<()> {
        let ([old], [new]) = (old, new) else { return Ok(()) };
        let Some(value) = block_value(old) else {
            return Ok(());
        };
        if new.is_empty() {
            return Ok(());
        }
        self.write(Change::Ids(crate::pool::IdsChange {
            replaced: vec![(value, new.clone())],
            ..Default::default()
        }))
    }

    /// Forget every alias naming one of `ids` (paragraphs an edit joined away).
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub(crate) fn forget_alias_targets(&mut self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        self.write(Change::Ids(crate::pool::IdsChange {
            forget_targets: ids.to_vec(),
            ..Default::default()
        }))
    }

    /// Replace a range across `paragraphs` (in order, children of `work`'s root) -- from
    /// `start` in the first to `end` in the last -- with `text`, joining them as Word joins
    /// paragraphs whose marks are deleted, by the policy ([`crate::across`]).
    ///
    /// # Errors
    /// [`EditError::Text`] or [`EditError::Span`] for a range the paragraphs cannot take.
    pub(crate) fn replace_across(
        &mut self,
        work: &mut Work,
        paragraphs: &[crate::work::Id],
        span: (usize, usize),
        text: &str,
        keep: Keep,
    ) -> Result<TextWrite> {
        write::check_text(text)?;
        let shared = self.doc().shared_handle();
        let orders = shared.facts.orders();
        match self.policy.clone() {
            RevisionPolicy::Untracked => Ok(TextWrite {
                released: crate::across::replace_across(work, paragraphs, span, text, keep)?,
                changed: work.changed(),
                dated: false,
                annotations: None,
            }),
            RevisionPolicy::Tracked(tracking) => {
                let (released, dated, numbered) = self.tracked(tracking, |stamp| {
                    crate::across::replace_across_tracked(work, paragraphs, span, text, keep, stamp, orders)
                })?;
                Ok(TextWrite {
                    changed: work.changed(),
                    released,
                    dated,
                    annotations: Some(numbered),
                })
            }
        }
    }

    /// Record `paragraph` of `work`, just put in place, as the policy writes a new
    /// paragraph: untracked as it is; tracked as inserted ([`crate::marks::track_new_paragraph`];
    /// `last`: no block follows it in its container).
    ///
    /// # Errors
    /// When the annotation ids cannot be read.
    pub(crate) fn new_paragraph(
        &mut self,
        work: &mut Work,
        paragraph: crate::work::Id,
        last: bool,
    ) -> Result<TextWrite> {
        let RevisionPolicy::Tracked(tracking) = self.policy.clone() else {
            return Ok(TextWrite::default());
        };
        let shared = self.doc().shared_handle();
        let orders = shared.facts.orders();
        let (released, dated, numbered) = self.tracked(tracking, |stamp| {
            crate::marks::track_new_paragraph(work, paragraph, last, stamp, orders);
            Ok(Vec::new())
        })?;
        Ok(TextWrite {
            changed: true,
            released,
            dated,
            annotations: Some(numbered),
        })
    }

    /// Record that `paragraph`'s properties were `old` (a detached clean copy) before the
    /// edit, by the policy: untracked nothing; tracked a `w:pPrChange`
    /// ([`crate::marks::record_paragraph_change`], docx-agent's `_track_properties`).
    ///
    /// # Errors
    /// When the annotation ids cannot be read.
    pub(crate) fn paragraph_properties_changed(
        &mut self,
        work: &mut Work,
        paragraph: crate::work::Id,
        old: crate::work::Id,
    ) -> Result<TextWrite> {
        let RevisionPolicy::Tracked(tracking) = self.policy.clone() else {
            return Ok(TextWrite::default());
        };
        let shared = self.doc().shared_handle();
        let orders = shared.facts.orders();
        let (released, dated, numbered) = self.tracked(tracking, |stamp| {
            crate::marks::record_paragraph_change(work, paragraph, old, stamp, orders);
            Ok(Vec::new())
        })?;
        Ok(TextWrite {
            changed: true,
            released,
            dated,
            annotations: Some(numbered),
        })
    }

    /// Read the formatting of every paragraph and run at or below `roots` of `work` before
    /// an edit formats them, when the policy records formatting changes (tracked); `None`
    /// untracked, where nothing is recorded (docx-agent's `_track_properties`).
    pub(crate) fn watch_properties(&self, work: &mut Work, roots: &[crate::work::Id]) -> Option<PropertyTracker> {
        matches!(self.policy, RevisionPolicy::Tracked(_)).then(|| PropertyTracker::start(work, roots))
    }

    /// Record the formatting changes an edit made under `tracker`'s roots
    /// ([`EditSink::watch_properties`]): each paragraph's in a `w:pPrChange`, its mark's and
    /// each run's in a `w:rPrChange` ([`PropertyTracker::record`]).
    ///
    /// # Errors
    /// When the annotation ids cannot be read.
    pub(crate) fn properties_changed(
        &mut self,
        work: &mut Work,
        tracker: Option<PropertyTracker>,
    ) -> Result<TextWrite> {
        let (Some(tracker), RevisionPolicy::Tracked(tracking)) = (tracker, self.policy.clone()) else {
            return Ok(TextWrite::default());
        };
        let shared = self.doc().shared_handle();
        let orders = shared.facts.orders();
        let (released, dated, numbered) = self.tracked(tracking, |stamp| {
            tracker.record(work, stamp, orders);
            Ok(Vec::new())
        })?;
        Ok(TextWrite {
            changed: true,
            released,
            dated,
            annotations: Some(numbered),
        })
    }

    /// Record the deletion of `paragraph` of `work` by the policy (tracked only: an
    /// untracked deletion removes it outright): its content and mark, or -- `previous`,
    /// the paragraph before it when it is last in its container -- that one's mark, this
    /// one taking its properties ([`crate::delete_write::track_delete_paragraph`]).
    ///
    /// # Errors
    /// When the annotation ids cannot be read.
    pub(crate) fn delete_paragraph(
        &mut self,
        work: &mut Work,
        paragraph: crate::work::Id,
        previous: Option<crate::work::Id>,
    ) -> Result<TextWrite> {
        let RevisionPolicy::Tracked(tracking) = self.policy.clone() else {
            return Ok(TextWrite::default());
        };
        let shared = self.doc().shared_handle();
        let orders = shared.facts.orders();
        let (released, dated, numbered) = self.tracked(tracking, |stamp| {
            Ok(crate::delete_write::track_delete_paragraph(
                work, paragraph, previous, stamp, orders,
            ))
        })?;
        Ok(TextWrite {
            changed: true,
            released,
            dated,
            annotations: Some(numbered),
        })
    }

    /// A tracked write of `work` by `write` (given the edit's [`Stamp`] and the schema's
    /// child order; it returns the relationship ids released); untracked, nothing.
    ///
    /// # Errors
    /// When the annotation ids cannot be read.
    pub(crate) fn track(
        &mut self,
        work: &mut Work,
        write: impl FnOnce(&mut Work, &mut Stamp, &crate::validate::Orders) -> Vec<String>,
    ) -> Result<TextWrite> {
        let RevisionPolicy::Tracked(tracking) = self.policy.clone() else {
            return Ok(TextWrite::default());
        };
        let shared = self.doc().shared_handle();
        let orders = shared.facts.orders();
        let (released, dated, numbered) = self.tracked(tracking, |stamp| Ok(write(work, stamp, orders)))?;
        Ok(TextWrite {
            changed: true,
            released,
            dated,
            annotations: Some(numbered),
        })
    }

    /// Add a part as docx-agent's package adds one (ooxml-edit's `add_part` with an
    /// `Override`, `add_relationship`): named `default`, or `template` numbered when that
    /// is taken, its tree `root`, its content type declared, related from `main` by
    /// `rel_type`; its name.
    ///
    /// # Errors
    /// When the content types or relationships are not XML.
    pub(crate) fn new_part(
        &mut self,
        main: &str,
        (default, template): (&str, &str),
        root: Element,
        content_type: &str,
        rel_type: &str,
    ) -> Result<String> {
        let package = self.doc().package().clone();
        let part = crate::new_part::unused_name(&package, default, template);
        let made = crate::new_part::add_part(&package, main, &part, root, content_type, rel_type)?;
        let rels = ooxml_opc::name::rels_for(main);
        for name in [part.as_str(), ooxml_opc::CONTENT_TYPES_PART, rels.as_str()] {
            if let Some(tree) = made.tree(name)? {
                self.put(name, tree)?;
            }
        }
        Ok(part)
    }

    /// Replace `count` children of the element at `parent` (child indexes from the root)
    /// of `part`, from its child `start` on, by `nodes`; ids drawn into them where the
    /// change is applied.
    ///
    /// # Errors
    /// When the part has no such element or children.
    pub(crate) fn splice_children(
        &mut self,
        part: &str,
        (parent, start, count): (&[u32], usize, usize),
        nodes: Vec<Node>,
        keys: Vec<Key>,
        draws: Vec<Draw>,
    ) -> Result<()> {
        let tree = self
            .tx
            .doc()
            .tree(part)?
            .ok_or_else(|| EditError::NoPart(part.to_owned()))?;
        let element =
            ooxml_word_model::docx::at_path(tree.root(), parent).ok_or_else(|| EditError::Path(part.to_owned()))?;
        let removed: Vec<Node> = element.children().iter().skip(start).take(count).cloned().collect();
        if removed.len() != count {
            return Err(EditError::Path(part.to_owned()));
        }
        let mut parent_path = vec![0usize];
        parent_path.extend(parent.iter().map(|i| *i as usize));
        self.write(Change::Splice {
            part: Arc::from(part),
            patch: Patch::splice(NodePath(parent_path), start, removed, nodes),
            keys: keys.into(),
            draws: draws.into(),
            created: Arc::from([]),
        })
    }

    /// Replace the current view's characters `[start, end)` of the paragraph `work` holds
    /// with `text`, by the policy (docx-agent's `inline.replace_span`, tracked
    /// `track.replace_span`).
    ///
    /// # Errors
    /// [`EditError::Text`] or [`EditError::Span`] for a span or text the paragraph cannot
    /// take.
    pub(crate) fn replace_span(
        &mut self,
        work: &mut Work,
        span: (usize, usize),
        text: &str,
        keep: Keep,
    ) -> Result<TextWrite> {
        let paragraph = work.root();
        write::check_text(text)?;
        match self.policy.clone() {
            RevisionPolicy::Untracked => Ok(TextWrite {
                released: crate::inline::replace_span(work, paragraph, span, text, keep)?,
                changed: work.changed(),
                dated: false,
                annotations: None,
            }),
            RevisionPolicy::Tracked(tracking) => {
                let (released, dated, numbered) = self.tracked(tracking, |stamp| {
                    track::replace_span(work, paragraph, span, text, keep, stamp)
                })?;
                Ok(TextWrite {
                    changed: work.changed(),
                    released,
                    dated,
                    annotations: Some(numbered),
                })
            }
        }
    }

    /// Drop `part`'s relationships `ids` where nothing in the part uses them any more, and
    /// the parts only they led to (ooxml-edit's `release` and `reap`): what an edit that
    /// removed content outright (a tracked removal of one's own insertion) leaves unused.
    /// The part is read whole (any attribute may hold an id).
    ///
    /// # Errors
    /// When a part is not XML.
    pub fn release(&mut self, part: &str, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        self.tx.read(Key::Part(Arc::from(part)));
        let released = crate::release::release(self.doc().package(), part, ids)?;
        for name in &released.changed {
            if let Some(tree) = released.package.tree(name)? {
                self.put(name, tree)?;
            }
        }
        for name in &released.removed {
            if self.doc().package().has_part(name) {
                self.write(Change::Remove {
                    part: Arc::from(name.as_str()),
                })?;
            }
        }
        Ok(())
    }

    /// Keep the people part where the change is applied: `add` listed, then `tidy` dropped
    /// if nothing of theirs is left ([`Change::People`]).
    ///
    /// # Errors
    /// When the main part's relationships are not XML.
    pub fn people(&mut self, add: Option<&str>, tidy: Option<&str>) -> Result<()> {
        let main = self.base().docx()?.main.clone();
        let mut keys: Vec<Key> = crate::people::parts_written(self.doc().package(), &main)
            .map_err(EditError::from)?
            .into_iter()
            .map(Key::Part)
            .collect();
        keys.push(Key::People);
        self.write(Change::People {
            add: add.map(Arc::from),
            tidy: tidy.map(Arc::from),
            keys: keys.into(),
        })
    }

    /// Replace the element at `path` (child indexes from the root) of `part` by `new`.
    ///
    /// # Errors
    /// When the part has no element there.
    pub fn replace(&mut self, part: &str, path: &[u32], new: Arc<Element>, keys: Vec<Key>) -> Result<()> {
        self.replace_drawing(part, path, new, keys, Vec::new())
    }

    /// [`EditSink::replace`], with ids drawn from the ledger into the new element where
    /// the change is applied ([`Draw`]): drawing is then no conflict between transactions.
    ///
    /// # Errors
    /// When the part has no element there.
    pub fn replace_drawing(
        &mut self,
        part: &str,
        path: &[u32],
        new: Arc<Element>,
        keys: Vec<Key>,
        draws: Vec<Draw>,
    ) -> Result<()> {
        let tree = self
            .tx
            .doc()
            .tree(part)?
            .ok_or_else(|| EditError::NoPart(part.to_owned()))?;
        let (&last, above) = path.split_last().ok_or_else(|| EditError::Path(part.to_owned()))?;
        let parent =
            ooxml_word_model::docx::at_path(tree.root(), above).ok_or_else(|| EditError::Path(part.to_owned()))?;
        let old = parent
            .children()
            .get(last as usize)
            .cloned()
            .ok_or_else(|| EditError::Path(part.to_owned()))?;
        let mut parent_path = vec![0usize];
        parent_path.extend(above.iter().map(|i| *i as usize));
        let patch = Patch::splice(
            NodePath(parent_path),
            last as usize,
            vec![old],
            vec![Node::Element(new)],
        );
        self.write(Change::Splice {
            part: Arc::from(part),
            patch,
            keys: keys.into(),
            draws: draws.into(),
            created: Arc::from([]),
        })
    }

    /// Change one element's head in place (its children kept).
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub fn set_head(&mut self, part: &str, patch: HeadPatch) -> Result<()> {
        self.set_head_keyed(part, patch, vec![Key::Part(Arc::from(part))])
    }

    /// Change one element's head in place, writing `keys`.
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub(crate) fn set_head_keyed(&mut self, part: &str, patch: HeadPatch, keys: Vec<Key>) -> Result<()> {
        self.write(Change::Head {
            part: Arc::from(part),
            patch,
            keys: keys.into(),
        })
    }

    /// Put `part` in place whole (a new part, or one rewritten throughout).
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub fn put(&mut self, part: &str, tree: ooxml_xml::Tree) -> Result<()> {
        // the tree holds the values this edit drew into the part as the version it was
        // prepared on gives them; applied to a newer head, those draws give other values
        // and the put would overwrite them: such an edit commits only where it was prepared
        let drawn = self
            .tx
            .changes()
            .iter()
            .any(|c| matches!(c, Change::Splice { part: p, draws, .. } if **p == *part && !draws.is_empty()));
        if drawn {
            self.tx.read_everything();
        }
        self.write(Change::Put {
            part: Arc::from(part),
            tree,
        })
    }

    /// Declare `w14` (and `wp14` with `drawings`, and each of `extra`) on `part`'s root and
    /// list them in `mc:Ignorable` (docx-agent's `ensure_w14`).
    ///
    /// # Errors
    /// When the part is not XML.
    pub fn ensure_w14(&mut self, part: &str, drawings: bool, extra: &[(&str, &str)]) -> Result<()> {
        let Some(tree) = self.tx.doc().tree(part)? else {
            return Ok(());
        };
        // what the root declares decides what is written (nothing, when it declares them)
        self.tx.read(Key::Root(Arc::from(part)));
        let root = tree.root();
        let Some((head, declared)) = ensured_head(root, drawings, extra) else {
            return Ok(());
        };
        let head = Arc::new(head);
        if declared {
            // lxml declares them with a cleanup of the whole part: declarations below that
            // repeat one in scope go (the part rewritten only when one does)
            let new = Arc::new(Element::from_head(Arc::clone(&head), root.children().clone()));
            let cleaned = crate::namespaces::cleaned(&new);
            if !Arc::ptr_eq(&cleaned, &new) {
                return self.put(part, tree.with_root(cleaned));
            }
        }
        let patch = HeadPatch::new(NodePath::root(), Arc::clone(root.head()), head);
        self.write(Change::Head {
            part: Arc::from(part),
            patch,
            keys: vec![Key::Root(Arc::from(part))].into(),
        })
    }

    /// Give the paragraph `work` holds the text `text` in the current view, by the policy:
    /// untracked, an in-place diff keeping each surviving character's formatting; tracked,
    /// each changed stretch deleted and inserted at word boundaries by the tracking's author
    /// (revision ids allocated here, the part's root then declaring `w16du`).
    ///
    /// # Errors
    /// [`EditError::Text`] or [`EditError::Span`] for text the paragraph cannot take.
    pub(crate) fn set_paragraph_text(&mut self, work: &mut Work, text: &str) -> Result<TextWrite> {
        let paragraph = work.root();
        match self.policy.clone() {
            RevisionPolicy::Untracked => Ok(TextWrite {
                changed: write::set_text(work, paragraph, text)?,
                released: Vec::new(),
                dated: false,
                annotations: None,
            }),
            RevisionPolicy::Tracked(tracking) => {
                write::check_text(text)?;
                let length = crate::text::atoms(work, paragraph).len();
                let (released, dated, numbered) = self.tracked(tracking, |stamp| {
                    track::replace_span(work, paragraph, (0, length), text, Keep::Characters, stamp)
                })?;
                Ok(TextWrite {
                    changed: work.changed(),
                    released,
                    dated,
                    annotations: Some(numbered),
                })
            }
        }
    }
}

/// The value of the block id `id` names (`p:`, `t:`, `tr:` or `s:` and eight hex digits,
/// in a story or not), as docx-agent's `_GONE_BLOCK` reads one.
fn block_value(id: &str) -> Option<u32> {
    let own = id.rsplit('/').next()?;
    let (kind, hex) = own.split_once(':')?;
    if hex.len() != 8 || !matches!(kind, "p" | "t" | "tr" | "s") {
        return None;
    }
    ooxml_word_model::ids::hex_value(hex)
}
