//! Owns: [`InsertNote`], a footnote or endnote inserted as docx-agent's `_insert_note`
//! inserts one: its reference a run in the reference style at a position (or a range's
//! end) in the body, the note -- numbered one above the largest in its part -- a paragraph
//! per line in the text style, the first opening with the number mark and a space; the
//! notes parts made with the document's first note. Tracked, the reference's run and the
//! note's content are insertions, and each of its paragraphs' marks but the last.
//!
//! Must not: write but through the sink.

use super::{ensure_notes_part, next_number, paragraph_key, place, stamp_paragraph, NoteEdited, NoteKind};
use crate::doc::{Draw, DrawFrom, Key};
use crate::links::set_run_style;
use crate::names::{W, W14, W16DU};
use crate::policy::RevisionPolicy;
use crate::review::space::Space;
use crate::review::xml::make;
use crate::sink::{annotation_draws, EditSink};
use crate::stamping::{stamp_document, w14_prefix};
use crate::text::write::{check_text, make_run};
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::at_path;
use ooxml_xml::QName;
use std::sync::Arc;

/// A note inserted.
#[derive(Debug, Clone)]
pub struct InsertNote {
    /// A footnote or an endnote.
    pub kind: NoteKind,
    /// Where its reference goes: a position, or a range (at its end).
    pub at: String,
    /// Its text, a line per paragraph.
    pub text: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

impl InsertNote {
    fn check(&self, doc: &WordDoc) -> Result<()> {
        check_text(&self.text.replace('\n', ""))?;
        let docx = doc.docx()?;
        place(doc, &docx, &self.at)?;
        let (text_style, reference_style) = self.kind.styles();
        crate::builtin::resolve(&docx, text_style, "paragraph")?;
        crate::builtin::resolve(&docx, reference_style, "character")?;
        Ok(())
    }
}

impl Command<WordDoc> for InsertNote {
    type Out = NoteEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.check(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<NoteEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        self.check(sink.doc())?;
        let docx = sink.docx()?;
        let (paragraph, offset) = place(sink.doc(), &docx, &self.at)?;
        sink.read(paragraph_key(&paragraph));
        sink.read(Key::Annotations);
        let renamed = stamp_document(&mut sink, &docx)?;
        let (text_style, reference_style) = self.kind.styles();
        let text_id = crate::builtin::ensure(&mut sink, text_style, "paragraph")?;
        let reference_id = crate::builtin::ensure(&mut sink, reference_style, "character")?;
        let notes = ensure_notes_part(&mut sink, self.kind)?;
        sink.read(Key::Part(Arc::from(notes.as_str())));
        let (mut space, paragraphs, number, ids) =
            self.new_note(&mut sink, &docx, &notes, (&text_id, &reference_id))?;
        let name = self.kind.name();
        // the reference, in the body paragraph
        let main = paragraph.part.clone();
        let tree = sink.doc().tree(&main)?.ok_or_else(|| EditError::NoPart(main.clone()))?;
        let element = at_path(tree.root(), &paragraph.path).ok_or_else(|| EditError::Path(main.clone()))?;
        let mut body = Work::of(element, scope_above(tree.root(), &paragraph.path));
        let body_root = body.root();
        let reference = make(&mut body, &format!("{name}Reference"), &[("id", &number.to_string())]);
        let run = crate::inline::new_run(&mut body, body_root, offset, &[reference])?;
        let shared = sink.doc().shared_handle();
        set_run_style(&mut body, run, &reference_id, shared.facts.orders());
        let (numbered, dated) = track(
            &mut sink,
            &mut body,
            run,
            (&mut space, &notes, &paragraphs),
            shared.facts.orders(),
        )?;
        space.finish(&mut sink)?;
        sink.ensure_w14(&notes, false, &[])?;
        let new = body.build().ok_or_else(|| EditError::Path(main.clone()))?;
        let prefix = w14_prefix(tree.root(), &paragraph.path);
        let mut draws = numbered.map(|n| annotation_draws(&new, n, 0)).unwrap_or_default();
        draws.push(Draw {
            from: DrawFrom::Seed(Arc::from(format!("{main}\0text"))),
            node: 0,
            at: Vec::new(),
            attribute: Arc::new(QName::new(Some(&prefix), "textId", Some(W14))),
        });
        let id = crate::ranges::renamed_id(&renamed, &paragraph.id);
        let keys = vec![crate::set_text::block_key(&main, &id), Key::Annotations];
        sink.replace_drawing(&main, &paragraph.path, new, keys, draws)?;
        if dated {
            for part in [main.as_str(), notes.as_str()] {
                sink.ensure_w14(part, false, &[("w16du", W16DU)])?;
            }
        }
        if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
            sink.people(Some(&tracking.author), Some(&tracking.author))?;
        }
        let note_id = format!("{}:{number}", self.kind.prefix());
        let mut created = vec![note_id.clone()];
        created.extend(ids);
        Ok(NoteEdited {
            id: Some(note_id),
            created,
            removed: Vec::new(),
            renamed,
        })
    }
}

impl InsertNote {
    /// The note appended to its part's copy in a new [`Space`], its paragraphs stamped:
    /// the space, the paragraphs, the note's number and the paragraphs' ids.
    fn new_note(
        &self,
        sink: &mut EditSink<'_, '_>,
        docx: &ooxml_word_model::docx::Docx,
        notes: &str,
        (text_id, reference_id): (&str, &str),
    ) -> Result<(Space, Vec<Id>, i64, Vec<String>)> {
        let notes = notes.to_owned();
        let held = sink.doc().clone();
        let mut space = Space::default();
        let work = space
            .work(&held, &notes)?
            .ok_or_else(|| EditError::NoPart(notes.clone()))?;
        let root = work.root();
        let number = {
            let tree = held.tree(&notes)?.ok_or_else(|| EditError::NoPart(notes.clone()))?;
            next_number(tree.root(), self.kind)
        };
        let name = self.kind.name();
        let note = make(work, name, &[("id", &number.to_string())]);
        let paragraphs = note_paragraphs(work, self.kind, &self.text, text_id, reference_id);
        for p in &paragraphs {
            work.append(note, *p);
        }
        work.append(root, note);
        for p in &paragraphs {
            stamp_paragraph(sink, work_of(&mut space, &notes)?, *p, &notes)?;
        }
        let story = docx
            .part(&notes)
            .map_or_else(|| name.to_owned() + "s", |s| s.story.clone());
        let work = work_of(&mut space, &notes)?;
        let ids: Vec<String> = paragraphs
            .iter()
            .map(|p| {
                let para = work.get(*p, Some(W14), "paraId").unwrap_or("").to_owned();
                format!("{story}/p:{para}")
            })
            .collect();
        Ok((space, paragraphs, number, ids))
    }
}

/// Tracked, the reference's run (`run` of `body`) and the note's content recorded as
/// inserted, and each of its paragraphs' marks but the last: the ids the body's records
/// took and whether any record is dated.
fn track(
    sink: &mut EditSink<'_, '_>,
    body: &mut Work,
    run: Id,
    (space, notes, paragraphs): (&mut Space, &str, &[Id]),
    orders: &crate::validate::Orders,
) -> Result<(Option<crate::sink::Numbered>, bool)> {
    let RevisionPolicy::Tracked(tracking) = sink.policy().clone() else {
        return Ok((None, false));
    };
    let mut numbered = None;
    let mut dated = false;
    if let (Some(parent), Some(index)) = (body.parent(run), body.index(run)) {
        body.remove(run);
        let ((), n, d) = sink.stamped(tracking.clone(), |stamp| {
            crate::track::insert_nodes(body, parent, index, &[run], stamp);
            Ok(())
        })?;
        numbered = Some(n);
        dated |= d;
    }
    let work = work_of(space, notes)?;
    let count = paragraphs.len();
    let ((), _, d) = sink.stamped(tracking, |stamp| {
        for (k, p) in paragraphs.iter().enumerate() {
            crate::marks::insert_paragraph_content(work, *p, stamp);
            if k + 1 < count {
                crate::marks::mark(work, *p, "ins", stamp, orders);
            }
        }
        Ok(())
    })?;
    Ok((numbered, dated || d))
}

/// The copy of `part` in `space`.
fn work_of<'s>(space: &'s mut Space, part: &str) -> Result<&'s mut Work> {
    space.get_mut(part).ok_or_else(|| EditError::NoPart(part.to_owned()))
}

/// A note's paragraphs as Word writes them (docx-agent's `_note_paragraphs`): each in the
/// text style, the first opening with the number mark in the reference style and a space.
fn note_paragraphs(work: &mut Work, kind: NoteKind, text: &str, text_style: &str, reference_style: &str) -> Vec<Id> {
    let mut out = Vec::new();
    for (k, line) in text.split('\n').enumerate() {
        let paragraph = make(work, "p", &[]);
        let properties = make(work, "pPr", &[]);
        let style = make(work, "pStyle", &[("val", text_style)]);
        work.append(properties, style);
        work.append(paragraph, properties);
        if k == 0 {
            let mark = make(work, "r", &[]);
            let run_properties = make(work, "rPr", &[]);
            let reference = make(work, "rStyle", &[("val", reference_style)]);
            work.append(run_properties, reference);
            work.append(mark, run_properties);
            let number = make(work, &format!("{}Ref", kind.name()), &[]);
            work.append(mark, number);
            work.append(paragraph, mark);
            let space = make_run(work, " ", None);
            work.append(paragraph, space);
        }
        if !line.is_empty() {
            let run = make_run(work, line, None);
            work.append(paragraph, run);
        }
        out.push(paragraph);
    }
    let _ = W;
    out
}
