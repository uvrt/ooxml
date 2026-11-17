//! Owns: [`EditSink`], the one seam through which every Word edit writes (design section
//! 4): an element replaced by its edited copy, a root's head changed, a part put in place,
//! ids drawn from the ledger, annotation ids allocated, renames recorded -- each one a
//! primitive of the transaction's change set with the keys it reads and writes; and the
//! text primitive the [`RevisionPolicy`] turns into untracked or tracked markup
//! ([`EditSink::set_paragraph_text`]).
//!
//! Must not: be bypassed (no edit touches a tree any other way), commit (the store does),
//! or decide what an edit means.

use crate::doc::{Change, Draw, DrawFrom, Key, WordDoc};
use crate::names::{MC, W14, WP14};
use crate::policy::{RevisionPolicy, Stamp};
use crate::text::write;
use crate::track;
use crate::validate::annotation_ceiling;
use crate::work::Work;
use crate::{EditError, Result};
use ooxml_store::Txn;
use ooxml_word_model::docx::Docx;
use ooxml_xml::{Attribute, Element, Head, HeadPatch, NamespaceDecl, Node, NodePath, Patch, QName};
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
    /// The revision ids the edit gave, `first..next` (tracked): stand-ins, drawn again
    /// where the change is applied ([`crate::DrawFrom::Annotation`]).
    pub annotations: Option<(i64, i64)>,
}

/// The sink a command writes through, over one transaction.
pub struct EditSink<'t, 's> {
    tx: &'t mut Txn<'s, WordDoc>,
    policy: RevisionPolicy,
}

impl<'t, 's> EditSink<'t, 's> {
    /// A sink over `tx`, writing by `policy`.
    pub fn new(tx: &'t mut Txn<'s, WordDoc>, policy: RevisionPolicy) -> Self {
        EditSink { tx, policy }
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

    /// Record that the edit depends on `key`.
    pub fn read(&mut self, key: Key) {
        self.tx.read(key);
    }

    fn write(&mut self, change: Change) -> Result<()> {
        self.tx.write(change).map_err(EditError::from)
    }

    /// `count` new ids of `seed`, from the ledger (read and written: two transactions drawing
    /// from one seed are ordered by their commits).
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub fn draw(&mut self, seed: &str, count: usize) -> Result<Vec<String>> {
        self.tx.read(Key::Seq(Arc::from(seed)));
        let (values, change) = self.tx.doc().ids().draw(seed, count, &self.tx.doc().shared().sequences);
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

    /// One above the largest annotation id in use in any story of the working version: the
    /// first of the stand-in ids an edit numbers its revision records with, drawn again
    /// where the change is applied ([`crate::DrawFrom::Annotation`]), so revision ids
    /// are no conflict between transactions.
    ///
    /// # Errors
    /// When a story part is not XML.
    pub fn next_annotation_id(&mut self) -> Result<i64> {
        Ok(annotation_ceiling(self.tx.doc())? + 1)
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
                self.write(Change::Remove { part: Arc::from(name.as_str()) })?;
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
        })
    }

    /// Change one element's head in place (its children kept).
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub fn set_head(&mut self, part: &str, patch: HeadPatch) -> Result<()> {
        self.write(Change::Head {
            part: Arc::from(part),
            patch,
            keys: vec![Key::Part(Arc::from(part))].into(),
        })
    }

    /// Put `part` in place whole (a new part, or one rewritten throughout).
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub fn put(&mut self, part: &str, tree: ooxml_xml::Tree) -> Result<()> {
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
        let root = tree.root();
        let Some((head, declared)) = ensured_head(root, drawings, extra) else {
            return Ok(());
        };
        if declared {
            // lxml declares them with a cleanup of the whole part: declarations below that
            // repeat one in scope go
            let new = Arc::new(Element::from_head(Arc::new(head), root.children().clone()));
            return self.put(part, tree.with_root(crate::namespaces::cleaned(&new)));
        }
        let patch = HeadPatch::new(NodePath::root(), Arc::clone(root.head()), Arc::new(head));
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
                let first = self.next_annotation_id()?;
                let mut stamp = Stamp::new(tracking, first);
                let length = crate::text::atoms(work, paragraph).len();
                let released = track::replace_span(work, paragraph, 0, length, text, &mut stamp)?;
                Ok(TextWrite {
                    changed: work.changed(),
                    released,
                    dated: stamp.dated,
                    annotations: Some((first, stamp.next())),
                })
            }
        }
    }
}

/// `root`'s head with `w14` (`wp14`, `extra`) declared and listed as ignorable; `None` when
/// it has them already. New declarations go after the root's own, `mc` last when it is
/// added; the prefixes are added to the end of `mc:Ignorable`.
pub(crate) fn ensured_head(root: &Element, drawings: bool, extra: &[(&str, &str)]) -> Option<(Head, bool)> {
    let mut wanted: Vec<(&str, &str)> = vec![("w14", W14)];
    if drawings {
        wanted.push(("wp14", WP14));
    }
    wanted.extend_from_slice(extra);
    let declared = |namespaces: &[NamespaceDecl], uri: &str| {
        namespaces
            .iter()
            .find(|d| &*d.uri == uri && d.prefix.as_deref().is_some_and(|p| !p.is_empty()))
            .and_then(|d| d.prefix.as_deref().map(str::to_owned))
    };
    let mut namespaces = root.namespaces().to_vec();
    let mut changed = false;
    let mut found: Vec<String> = Vec::new();
    let missing_any = wanted.iter().any(|(_, uri)| declared(&namespaces, uri).is_none());
    let mc_missing = declared(&namespaces, MC).is_none();
    if missing_any || mc_missing {
        for (name, uri) in &wanted {
            if declared(&namespaces, uri).is_none() {
                namespaces.push(NamespaceDecl {
                    prefix: Some(Arc::from(*name)),
                    uri: Arc::from(*uri),
                });
                changed = true;
            }
        }
        if mc_missing && root.child_elements().next().is_some() {
            namespaces.push(NamespaceDecl {
                prefix: Some(Arc::from("mc")),
                uri: Arc::from(MC),
            });
            changed = true;
        }
    }
    for (name, uri) in &wanted {
        found.push(declared(&namespaces, uri).unwrap_or_else(|| (*name).to_owned()));
    }
    let mc = declared(&namespaces, MC).unwrap_or_else(|| "mc".to_owned());
    let mut attributes = root.attributes().to_vec();
    let listed: Vec<String> = root
        .attribute(Some(MC), "Ignorable")
        .map(|v| v.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default();
    let missing: Vec<&String> = found.iter().filter(|p| !listed.contains(p)).collect();
    if !missing.is_empty() {
        let value: Vec<&str> = listed
            .iter()
            .map(String::as_str)
            .chain(missing.iter().map(|s| s.as_str()))
            .collect();
        let value: Arc<str> = Arc::from(value.join(" "));
        match attributes.iter_mut().find(|a| a.name.is(Some(MC), "Ignorable")) {
            Some(a) => a.value = value,
            None => attributes.push(Attribute {
                name: Arc::new(QName::new(Some(&mc), "Ignorable", Some(MC))),
                value,
            }),
        }
        changed = true;
    }
    let declared_new = namespaces.len() != root.namespaces().len();
    changed.then(|| {
        let head = Head {
            name: root.name().clone(),
            namespaces: namespaces.into(),
            attributes: attributes.into(),
        };
        (head, declared_new)
    })
}

/// The draws that number again, where the change is applied, the revision records an
/// edit made in `element` (`w:id` in `first..next`, its stand-ins), each by its offset
/// from `first`; paths from `element`.
pub(crate) fn annotation_draws(element: &Element, first: i64, next: i64) -> Vec<Draw> {
    let mut out = Vec::new();
    let mut stack: Vec<(&Element, Vec<u32>)> = vec![(element, Vec::new())];
    while let Some((e, path)) = stack.pop() {
        if e.name().namespace() == Some(crate::names::W) && crate::revision_ids::is_record(e) {
            let value = e
                .attribute(Some(crate::names::W), "id")
                .and_then(|v| v.trim().parse::<i64>().ok())
                .filter(|v| (first..next).contains(v));
            if let Some(value) = value {
                if let (Ok(offset), Some(attribute)) = (
                    u32::try_from(value - first),
                    e.attributes().iter().find(|a| a.name.is(Some(crate::names::W), "id")),
                ) {
                    out.push(Draw {
                        from: DrawFrom::Annotation(offset),
                        at: path.clone(),
                        attribute: Arc::clone(&attribute.name),
                    });
                }
            }
        }
        for (k, node) in e.children().iter().enumerate() {
            if let Node::Element(child) = node {
                let mut below = path.clone();
                below.push(u32::try_from(k).unwrap_or(u32::MAX));
                stack.push((child, below));
            }
        }
    }
    out
}
