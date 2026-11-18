//! Owns: [`EditNote`], a note's text replaced as docx-agent's `edit_note` replaces it: its
//! paragraphs after the first deleted, the first one's text after the number mark (and the
//! space after it) replaced by the first line, a paragraph inserted after it for each
//! further line -- each step as the matching text edit writes it, tracked or not.
//!
//! Must not: write but through the sink.

use super::{note_element, NoteEdited};
use crate::names::W;
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::text::write::check_text;
use crate::text::{atoms, OBJECT};
use crate::work::Work;
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};

/// A note's text replaced.
#[derive(Debug, Clone)]
pub struct EditNote {
    /// The note (`fn:2`, `en:1`).
    pub note: String,
    /// Its new text, a line per paragraph.
    pub text: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

impl Command<WordDoc> for EditNote {
    type Out = NoteEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        check_text(&self.text.replace('\n', ""))?;
        note_element(doc, &*doc.docx()?, &self.note).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<NoteEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        check_text(&self.text.replace('\n', ""))?;
        let docx = sink.docx()?;
        let (part, path, _) = note_element(sink.doc(), &docx, &self.note)?;
        let story = docx.part(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
        let entries: Vec<_> = story
            .ids
            .paragraphs_within(&path)
            .into_iter()
            .filter(|p| p.element.name().is(Some(W), "p"))
            .collect();
        let Some(first) = entries.first() else {
            return Err(EditError::Refused(format!("{} has no paragraph", self.note)));
        };
        sink.read(crate::doc::Key::Part(std::sync::Arc::from(part.as_str())));
        let lines: Vec<&str> = self.text.split('\n').collect();
        let work = Work::of(&first.element, Vec::new());
        let items = atoms(&work, work.root());
        let mut start = items.iter().take_while(|a| a.ch == OBJECT).count();
        if items.get(start).is_some_and(|a| a.ch == ' ') {
            start += 1;
        }
        let mut renamed: Vec<(String, String)> = Vec::new();
        let mut created = Vec::new();
        let mut removed = Vec::new();
        let current = |renamed: &[(String, String)], id: &str| crate::ranges::renamed_id(renamed, id);
        for entry in entries.iter().skip(1) {
            let target = current(&renamed, &entry.id);
            let done = crate::delete_write::delete(&mut sink, &target)?;
            removed.extend(done.removed.clone());
            merge(&mut renamed, &done.renamed);
        }
        let first_id = current(&renamed, &first.id);
        let length = {
            let docx = sink.docx()?;
            let found = crate::set_text::find(sink.doc(), &docx, &first_id)?;
            let work = Work::of(&found.element, Vec::new());
            atoms(&work, work.root()).len()
        };
        let address = format!("{first_id}@{start}:{length}");
        let done = crate::ranges::replace_range(&mut sink, &address, lines.first().copied().unwrap_or(""))?;
        merge(&mut renamed, &done.renamed);
        let mut previous = current(&renamed, &first_id);
        for line in lines.iter().skip(1) {
            let done = crate::paragraphs::insert_paragraph(&mut sink, &previous, (true, line, None))?;
            created.push(done.id.clone());
            merge(&mut renamed, &done.renamed);
            previous = done.id;
        }
        Ok(NoteEdited {
            id: Some(self.note.clone()),
            created,
            removed,
            renamed,
        })
    }
}

/// `renamed` updated by `more` as a dict update: a key already there keeps its place.
fn merge(renamed: &mut Vec<(String, String)>, more: &[(String, String)]) {
    for (old, new) in more {
        match renamed.iter_mut().find(|(o, _)| o == old) {
            Some(found) => found.1.clone_from(new),
            None => renamed.push((old.clone(), new.clone())),
        }
    }
}
