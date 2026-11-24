//! Owns: a note's reference moved while tracking changes, as docx-agent's `move_note` moves
//! one tracked. A reference has one note, so the reference is deleted where it was and a
//! copy of the note -- numbered one above the largest in its part, each of its paragraphs
//! given a fresh paraId and textId -- goes right after the note, with a new reference to it
//! at the new place in the old run's formatting. The new reference's run and the copy's
//! content are insertions, and each of the copy's paragraph marks but the last. One's own
//! inserted reference goes outright, and the old note with it.
//!
//! Must not: write but through the sink.

use super::change::{editable, paragraph_editable, put_reference, write_back, Editable, MoveNote};
use super::{isolate_reference, next_number, note_element, note_reference, stamp_paragraph, NoteEdited, NoteKind};
use crate::names::{W, W16DU};
use crate::policy::Tracking;
use crate::review::space::Space;
use crate::review::xml::{attached, make};
use crate::set_text::Found;
use crate::sink::{EditSink, Numbered};
use crate::stamping::stamp_document;
use crate::work::{Id, Work};
use crate::{EditError, Result};

impl MoveNote {
    /// The tracked move, the reference to go at `offset` of `paragraph`.
    pub(super) fn edit_tracked(
        &self,
        sink: &mut EditSink<'_, '_>,
        tracking: Tracking,
        paragraph: &Found,
        offset: usize,
    ) -> Result<NoteEdited> {
        let kind = NoteKind::of(&self.note);
        let docx = sink.docx()?;
        let renamed = stamp_document(&mut *sink, &docx)?;
        let docx = sink.docx()?;
        let no_reference = || EditError::Refused(format!("{} has no reference to move", self.note));
        let (old_part, reference) = note_reference(&docx, &self.note).ok_or_else(no_reference)?;
        let (notes, note_path, _) = note_element(sink.doc(), &docx, &self.note)?;
        let held = sink.doc().clone();
        let number = {
            let tree = held.tree(&notes)?.ok_or_else(|| EditError::no_part(notes.clone()))?;
            next_number(tree.root(), kind)
        };
        let mut space = Space::default();
        space.work(&held, &notes)?;
        let note = space
            .locate(&notes, &note_path)
            .ok_or_else(|| EditError::no_element(notes.clone()))?;
        let paragraphs = twin(sink, &mut space, &notes, note, number)?;
        let mut old = editable(&*sink, &old_part, &reference)?;
        let same = old.part == paragraph.part && old.path == paragraph.path.to_vec();
        let mut new = if same {
            None
        } else {
            Some(paragraph_editable(&*sink, &paragraph.part, &paragraph.path)?)
        };
        let run = new_reference(&mut old, new.as_mut(), kind, number, offset)?;
        let old_node = old.node;
        let (released, deleted, dated_old) = sink.stamped(tracking.clone(), |stamp| {
            let moved =
                isolate_reference(&mut old.work, old_node).ok_or_else(|| EditError::no_element(old.part.clone()))?;
            Ok(crate::track::delete_runs(&mut old.work, &[moved], stamp))
        })?;
        let gone = !attached(&old.work, old_node);
        let target = new.as_mut().unwrap_or(&mut old);
        let (inserted, dated_new) = track_inserted(sink, &tracking, &mut target.work, run)?;
        let notes_work = space.get_mut(&notes).ok_or_else(|| EditError::no_part(notes.clone()))?;
        if gone {
            notes_work.remove(note);
        }
        let dated_notes = track_copy(sink, &tracking, notes_work, &paragraphs)?;
        match &new {
            Some(new) => {
                write_back(&mut *sink, new, inserted, true)?;
                write_back(&mut *sink, &old, Some(deleted), false)?;
            }
            None => {
                let both = inserted.map(|n| Numbered {
                    from: deleted.from,
                    ..n
                });
                write_back(&mut *sink, &old, both.or(Some(deleted)), true)?;
            }
        }
        sink.release(&old_part, &released)?;
        space.finish(&mut *sink)?;
        sink.ensure_w14(&notes, false, &[])?;
        for (part, dated) in [
            (&old_part, dated_old),
            (&paragraph.part, dated_new),
            (&notes, dated_notes),
        ] {
            if dated {
                sink.ensure_w14(part, false, &[("w16du", W16DU)])?;
            }
        }
        sink.people(Some(&tracking.author), Some(&tracking.author))?;
        let id = format!("{}:{number}", kind.prefix());
        Ok(NoteEdited {
            id: Some(id.clone()),
            created: vec![id],
            removed: Vec::new(),
            renamed,
        })
    }
}

/// The note's copy, numbered `number`, put right after it; its paragraphs (in document
/// order) each given a paraId and textId drawn from the notes part's sequence.
fn twin(sink: &mut EditSink<'_, '_>, space: &mut Space, notes: &str, note: Id, number: i64) -> Result<Vec<Id>> {
    let work = space
        .get_mut(notes)
        .ok_or_else(|| EditError::no_part(notes.to_owned()))?;
    let copy = work.deep_copy(note);
    let id = work.qname(W, "id", "w");
    work.set(copy, id, &number.to_string());
    work.add_next(note, copy);
    let paragraphs = work.iter(copy, W, &["p"]);
    for p in &paragraphs {
        let work = space
            .get_mut(notes)
            .ok_or_else(|| EditError::no_part(notes.to_owned()))?;
        stamp_paragraph(sink, work, *p, notes)?;
    }
    Ok(paragraphs)
}

/// The new reference to note `number` in a run of its own at `offset` of the target
/// paragraph (`new`, or `old`'s own), in the old reference's run's formatting: the run.
fn new_reference(
    old: &mut Editable,
    new: Option<&mut Editable>,
    kind: NoteKind,
    number: i64,
    offset: usize,
) -> Result<Id> {
    let properties = old.work.parent(old.node).and_then(|run| old.work.find(run, W, "rPr"));
    let properties = properties.map(|p| old.work.build_element(p));
    let target = new.unwrap_or(old);
    let properties = match properties {
        Some(Some(element)) => Some(
            target
                .work
                .adopt(&element)
                .ok_or_else(|| EditError::no_element(target.part.clone()))?,
        ),
        Some(None) => return Err(EditError::no_element(target.part.clone())),
        None => None,
    };
    let reference = make(
        &mut target.work,
        &format!("{}Reference", kind.name()),
        &[("id", &number.to_string())],
    );
    let root = target.work.root();
    put_reference(&mut target.work, root, offset, reference, properties)
}

/// The note's copy recorded as inserted: its paragraphs' content, and each of their marks
/// but the last; whether a record is dated.
fn track_copy(sink: &mut EditSink<'_, '_>, tracking: &Tracking, work: &mut Work, paragraphs: &[Id]) -> Result<bool> {
    let orders = sink.doc().shared_handle();
    let ((), _, dated) = sink.stamped(tracking.clone(), |stamp| {
        for (k, p) in paragraphs.iter().enumerate() {
            crate::marks::insert_paragraph_content(work, *p, stamp);
            if k + 1 < paragraphs.len() {
                crate::marks::mark(work, *p, "ins", stamp, orders.facts.orders());
            }
        }
        Ok(())
    })?;
    Ok(dated)
}

/// docx-agent's `_track_inserted`: `run`, just placed, put inside an insertion where it
/// stands; the ids the record took and whether it is dated.
fn track_inserted(
    sink: &mut EditSink<'_, '_>,
    tracking: &Tracking,
    work: &mut Work,
    run: Id,
) -> Result<(Option<Numbered>, bool)> {
    let (Some(parent), Some(index)) = (work.parent(run), work.index(run)) else {
        return Ok((None, false));
    };
    work.remove(run);
    let ((), numbered, dated) = sink.stamped(tracking.clone(), |stamp| {
        crate::track::insert_nodes(work, parent, index, &[run], stamp);
        Ok(())
    })?;
    Ok((Some(numbered), dated))
}
