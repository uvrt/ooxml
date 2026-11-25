//! Owns: [`DeleteNote`] and [`MoveNote`], as docx-agent's `delete_note` and `move_note` do
//! them: a deletion takes the reference (with its run when it holds nothing else) and the
//! note out -- tracked, the reference is deleted as a revision and the note stays, unless
//! it was one's own insertion, which goes outright with its note; a move puts a copy of the
//! reference, in its run's formatting, at the new place and takes the old one out.
//!
//! A tracked move (a reference has one note: the reference deleted, a copy of the note
//! inserted) is [`super::moved`]'s.
//!
//! Must not: write but through the sink.

use super::{isolate_reference, note_element, note_reference, place, remove_reference, NoteEdited};
use crate::doc::{Draw, DrawFrom, Key};
use crate::names::{W, W14, W16DU};
use crate::policy::RevisionPolicy;
use crate::review::space::Space;
use crate::review::xml::attached;
use crate::sink::{annotation_draws, EditSink};
use crate::stamping::{stamp_document, w14_prefix};
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::at_path;
use ooxml_xml::{Element, QName};
use std::sync::Arc;

/// A note deleted with its reference.
#[derive(Debug, Clone)]
pub struct DeleteNote {
    /// The note (`fn:2`, `en:1`).
    pub note: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// A note's reference moved.
#[derive(Debug, Clone)]
pub struct MoveNote {
    /// The note (`fn:2`, `en:1`).
    pub note: String,
    /// Where its reference goes: a position, or a range (at its end).
    pub to: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// The place of the paragraph holding the element at `path` (the innermost), and the rest
/// of the path below it.
fn paragraph_of(root: &Arc<Element>, path: &[u32]) -> Option<(Vec<u32>, Vec<u32>)> {
    let mut element = root;
    let mut found = None;
    for (k, &index) in path.iter().enumerate() {
        element = match element.children().get(index as usize)? {
            ooxml_xml::Node::Element(e) => e,
            _ => return None,
        };
        if element.name().is(Some(W), "p") && k + 1 < path.len() {
            found = Some(k + 1);
        }
    }
    let at = found?;
    Some((path.get(..at)?.to_vec(), path.get(at..)?.to_vec()))
}

/// The node below `top` at `below` (child element indexes, text aside, in a work made of
/// the element at `top`).
fn descend(work: &Work, element: &Arc<Element>, below: &[u32]) -> Option<Id> {
    let mut at = work.root();
    let mut element = Arc::clone(element);
    for &k in below {
        let nodes = element.children().to_vec();
        let index = nodes
            .iter()
            .take(k as usize)
            .filter(|n| !matches!(n, ooxml_xml::Node::Text(_)))
            .count();
        at = *work.children(at).get(index)?;
        element = match nodes.get(k as usize)? {
            ooxml_xml::Node::Element(e) => Arc::clone(e),
            _ => return None,
        };
    }
    Some(at)
}

/// A paragraph made editable: its place, the work and the node at `below`.
pub(super) struct Editable {
    pub(super) part: String,
    pub(super) path: Vec<u32>,
    pub(super) work: Work,
    pub(super) node: Id,
}

/// The paragraph holding the element at `reference` of `part`, made editable.
pub(super) fn editable(sink: &EditSink<'_, '_>, part: &str, reference: &[u32]) -> Result<Editable> {
    let tree = sink
        .doc()
        .tree(part)?
        .ok_or_else(|| EditError::no_part(part.to_owned()))?;
    let path_error = || EditError::no_element(part.to_owned());
    let (path, below) = paragraph_of(tree.root(), reference).ok_or_else(path_error)?;
    let element = at_path(tree.root(), &path).ok_or_else(path_error)?;
    let work = Work::of(element, scope_above(tree.root(), &path));
    let node = descend(&work, element, &below).ok_or_else(path_error)?;
    Ok(Editable {
        part: part.to_owned(),
        path,
        work,
        node,
    })
}

/// The paragraph written back, its textId drawn again when `renew`.
pub(super) fn write_back(
    sink: &mut EditSink<'_, '_>,
    edited: &Editable,
    numbered: Option<crate::sink::Numbered>,
    renew: bool,
) -> Result<()> {
    let tree = sink
        .doc()
        .tree(&edited.part)?
        .ok_or_else(|| EditError::no_part(edited.part.clone()))?;
    let new = edited
        .work
        .build()
        .ok_or_else(|| EditError::unbuilt(edited.part.clone()))?;
    let mut draws = numbered.map(|n| annotation_draws(&new, n, 0)).unwrap_or_default();
    if renew && new.attribute(Some(W14), "paraId").is_some() {
        let prefix = w14_prefix(tree.root(), &edited.path);
        draws.push(Draw {
            from: DrawFrom::Seed(Arc::from(format!("{}\0text", edited.part))),
            node: 0,
            at: Vec::new(),
            attribute: Arc::new(QName::new(Some(&prefix), "textId", Some(W14))),
        });
    }
    let docx = sink.docx()?;
    let mut keys = vec![Key::Annotations];
    if let Some(p) = docx.part(&edited.part).and_then(|s| s.ids.paragraph_at(&edited.path)) {
        keys.push(crate::set_text::block_key(&edited.part, &p.id));
    }
    sink.replace_drawing(&edited.part, &edited.path, new, keys, draws)
}

/// The note's element removed from its part.
fn remove_note(sink: &mut EditSink<'_, '_>, id: &str) -> Result<()> {
    let docx = sink.docx()?;
    let (part, path, _) = note_element(sink.doc(), &docx, id)?;
    let held = sink.doc().clone();
    let mut space = Space::default();
    space.work(&held, &part)?;
    let node = space
        .locate(&part, &path)
        .ok_or_else(|| EditError::no_element(part.clone()))?;
    if let Some(work) = space.get_mut(&part) {
        work.remove(node);
    }
    space.finish(sink)
}

impl Command<WordDoc> for DeleteNote {
    type Out = NoteEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        note_element(doc, &*doc.docx()?, &self.note).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<NoteEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl DeleteNote {
    /// The edit, through `sink`.
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<NoteEdited> {
        sink.read_everything();
        let docx = sink.docx()?;
        let (notes_part, note_path, note) = note_element(sink.doc(), &docx, &self.note)?;
        let found = note_reference(&docx, &self.note);
        if let (RevisionPolicy::Tracked(tracking), Some((part, reference))) = (sink.policy().clone(), &found) {
            let renamed = stamp_document(&mut *sink, &docx)?;
            let mut edited = editable(&*sink, part, reference)?;
            let reference_node = edited.node;
            let (released, numbered, dated) = sink.stamped(tracking.clone(), |stamp| {
                let run = isolate_reference(&mut edited.work, reference_node)
                    .ok_or_else(|| EditError::no_element(edited.part.clone()))?;
                Ok(crate::track::delete_runs(&mut edited.work, &[run], stamp))
            })?;
            let gone = !attached(&edited.work, reference_node);
            write_back(&mut *sink, &edited, Some(numbered), false)?;
            sink.release(part, &released)?;
            if dated {
                sink.ensure_w14(part, false, &[("w16du", W16DU)])?;
            }
            // the people listed as docx-agent's stamp lists them, before the note goes: its
            // own records still name the author; the author listed only when the stamp
            // recorded something (one's own inserted reference goes outright, unrecorded)
            let recorded = numbered.to > numbered.from;
            sink.people(recorded.then_some(tracking.author.as_str()), Some(&tracking.author))?;
            if gone {
                remove_note(&mut *sink, &self.note)?;
                return Ok(NoteEdited {
                    id: None,
                    removed: vec![self.note.clone()],
                    renamed,
                    created: Vec::new(),
                });
            }
            return Ok(NoteEdited {
                id: Some(self.note.clone()),
                renamed,
                ..NoteEdited::default()
            });
        }
        let mut removed = vec![self.note.clone()];
        if let Some(story) = docx.part(&notes_part) {
            removed.extend(
                story
                    .ids
                    .paragraphs_within(&note_path)
                    .into_iter()
                    .filter(|p| p.element.name().is(Some(W), "p"))
                    .map(|p| p.id),
            );
        }
        let _ = note;
        let renamed = stamp_document(&mut *sink, &docx)?;
        if let Some((part, reference)) = &found {
            let mut edited = editable(&*sink, part, reference)?;
            let node = edited.node;
            remove_reference(&mut edited.work, node);
            write_back(&mut *sink, &edited, None, false)?;
        }
        remove_note(&mut *sink, &self.note)?;
        Ok(NoteEdited {
            id: None,
            renamed: renamed
                .into_iter()
                .filter(|(old, _)| !removed.iter().any(|r| r == old))
                .collect(),
            removed,
            created: Vec::new(),
        })
    }
}

impl Command<WordDoc> for MoveNote {
    type Out = NoteEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let docx = doc.docx()?;
        if note_reference(&docx, &self.note).is_none() {
            return Err(EditError::Refused(format!("{} has no reference to move", self.note)));
        }
        place(doc, &docx, &self.to).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<NoteEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl MoveNote {
    /// The edit, through `sink`.
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<NoteEdited> {
        sink.read_everything();
        let docx = sink.docx()?;
        if note_reference(&docx, &self.note).is_none() {
            return Err(EditError::Refused(format!("{} has no reference to move", self.note)));
        }
        let (paragraph, offset) = place(sink.doc(), &docx, &self.to)?;
        if let RevisionPolicy::Tracked(tracking) = &self.policy {
            return self.edit_tracked(sink, tracking.clone(), &paragraph, offset);
        }
        let renamed = stamp_document(&mut *sink, &docx)?;
        let docx = sink.docx()?;
        let (old_part, reference) = note_reference(&docx, &self.note)
            .ok_or_else(|| EditError::Refused(format!("{} has no reference to move", self.note)))?;
        let mut old = editable(&*sink, &old_part, &reference)?;
        let same = old.part == paragraph.part && old.path == paragraph.path.to_vec();
        let mut new = if same {
            None
        } else {
            Some(paragraph_editable(&*sink, &paragraph.part, &paragraph.path)?)
        };
        let old_node = old.node;
        let properties = old.work.parent(old_node).and_then(|run| old.work.find(run, W, "rPr"));
        let properties = properties.map(|p| old.work.deep_copy(p));
        let reference_copy = old.work.deep_copy(old_node);
        match new.as_mut() {
            None => {
                let root = old.work.root();
                put_reference(&mut old.work, root, offset, reference_copy, properties)?;
            }
            Some(target) => {
                let path_error = || EditError::no_element(old.part.clone());
                let copied = import(&mut target.work, &old.work, reference_copy).ok_or_else(path_error)?;
                let properties = match properties {
                    Some(p) => Some(import(&mut target.work, &old.work, p).ok_or_else(path_error)?),
                    None => None,
                };
                put_reference(&mut target.work, target.node, offset, copied, properties)?;
            }
        }
        remove_reference(&mut old.work, old_node);
        match &new {
            Some(new) => {
                write_back(&mut *sink, new, None, true)?;
                write_back(&mut *sink, &old, None, false)?;
            }
            None => write_back(&mut *sink, &old, None, true)?,
        }
        Ok(NoteEdited {
            id: Some(self.note.clone()),
            renamed,
            ..NoteEdited::default()
        })
    }
}

/// The paragraph at `path` of `part` made editable, its root the node.
pub(super) fn paragraph_editable(sink: &EditSink<'_, '_>, part: &str, path: &[u32]) -> Result<Editable> {
    let tree = sink
        .doc()
        .tree(part)?
        .ok_or_else(|| EditError::no_part(part.to_owned()))?;
    let element = at_path(tree.root(), path).ok_or_else(|| EditError::no_element(part.to_owned()))?;
    let work = Work::of(element, scope_above(tree.root(), path));
    let root = work.root();
    Ok(Editable {
        part: part.to_owned(),
        path: path.to_vec(),
        work,
        node: root,
    })
}

/// A reference's copy put in a run of its own at `offset` of `paragraph`, in the old
/// run's formatting (`properties`) where it had any: the run.
pub(super) fn put_reference(
    work: &mut Work,
    paragraph: Id,
    offset: usize,
    reference: Id,
    properties: Option<Id>,
) -> Result<Id> {
    let run = crate::inline::new_run(work, paragraph, offset, &[reference])?;
    if let Some(properties) = properties {
        if let Some(existing) = work.find(run, W, "rPr") {
            work.remove(existing);
        }
        work.insert(run, 0, properties);
    }
    Ok(run)
}

/// `node` of `from` copied into `into` (detached).
pub(super) fn import(into: &mut Work, from: &Work, node: Id) -> Option<Id> {
    into.adopt(&from.build_element(node)?)
}
