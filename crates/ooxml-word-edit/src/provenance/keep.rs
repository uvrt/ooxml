//! Owns: keeping values written by reference, and labels written in place of values, on
//! their paragraphs ([`KeepText`]: docx-agent's `keep_text_facts`, `keep_text_facts_in`
//! and `keep_text_label`, each `_keep_text`): per paragraph, in order, the new facts and
//! labels that are where they say in its text now, beside the ones it kept that they do
//! not overlap -- the entry written, made (with the item, where the document has none) or
//! dropped when nothing is left.
//!
//! Reads: each paragraph kept on (its text), the main part's relationships and the item.
//!
//! Must not: find where a value went (the tool that wrote it says), or follow the other
//! entries (every command's follow-up does, [`super::Followed`]).

use super::item::Draft;
use super::made::make_item;
use crate::doc::{Key, WordDoc};
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_provenance::dumps;
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::provenance::keep::{kept, Kept};
use ooxml_word_model::docx::provenance::{kind_of, resolve_followed, Kind, TextEntry};
use ooxml_word_model::docx::text::{paragraph_text, View};
use ooxml_word_model::ids::Entry;
use serde_json::Value;

/// The kind of a paragraph's entry.
const TEXT: &str = "text";

/// What is kept on one paragraph.
#[derive(Debug, Clone, PartialEq)]
pub struct TextKeep {
    /// The paragraph (any address of one; its durable id is the entry's key).
    pub paragraph: String,
    /// Facts (`ooxml_edit.provenance` facts, `at` their place in the paragraph's text now).
    pub facts: Vec<Value>,
    /// Labels, as facts are given.
    pub labels: Vec<Value>,
}

/// Keep each of `keeps` on its paragraph, in order, in one version; its output says, per
/// keep, whether anything was kept (`false`: the paragraph has no durable id, or labels
/// were given and none is where it says).
#[derive(Debug, Clone, PartialEq)]
pub struct KeepText {
    /// The keeps, in order.
    pub keeps: Vec<TextKeep>,
}

impl Command<WordDoc> for KeepText {
    type Out = Vec<bool>;
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<()> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Vec<bool>> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        let docx = sink.docx()?;
        let aliases = sink.doc().ids().aliases().clone();
        let mut draft = Draft::open(&mut sink, &docx)?;
        let mut out = Vec::with_capacity(self.keeps.len());
        for keep in &self.keeps {
            let Some((part, Entry::Paragraph(k))) = resolve_followed(&docx, &aliases, &keep.paragraph) else {
                out.push(false);
                continue;
            };
            let Some(paragraph) = part.ids.paragraphs().get(k) else {
                out.push(false);
                continue;
            };
            if kind_of(&paragraph.id) != Some(Kind::Text) {
                out.push(false);
                continue;
            }
            sink.read(Key::block(part.part.as_str(), paragraph.id.as_str()));
            let now = paragraph_text(&paragraph.element, View::Current);
            let old = draft
                .as_ref()
                .and_then(|d| d.text_of(TEXT, &paragraph.id))
                .and_then(|text| TextEntry::read(&text));
            match kept(old.as_ref(), &now, &keep.facts, &keep.labels) {
                Kept::Refused => out.push(false),
                Kept::Cleared => {
                    if let Some(draft) = draft.as_mut() {
                        draft.drop_entry(TEXT, &paragraph.id);
                    }
                    out.push(true);
                }
                Kept::Entry(entry) => {
                    if draft.is_none() {
                        make_item(&mut sink, &docx, &paragraph.id)?;
                        let made = sink.docx()?;
                        draft = Draft::open(&mut sink, &made)?;
                    }
                    if let Some(draft) = draft.as_mut() {
                        draft.store(TEXT, &paragraph.id, &dumps(&entry.record()));
                    }
                    out.push(true);
                }
            }
        }
        if let Some(draft) = draft {
            draft.finish(&mut sink, &docx)?;
        }
        Ok(out)
    }
}
