//! Owns: [`SetStoryNumbered`], a header's or footer's text set with a page number after
//! it, as docx-agent's `word_headers_footers` writes one: `PAGE` at its last paragraph's
//! end; "Page X of Y" -- after `" | "` where the text does not end in a space, `Page  of `
//! typed, then `NUMPAGES` at its end and `PAGE` after `Page ` -- each field as Word
//! inserts it (`insert_page_number`: a complex `PAGE`, a simple `NUMPAGES`), its result
//! computed from the layout.
//!
//! Must not: write but through the sink.

use super::insert::InsertField;
use super::pages::FieldLayout;
use super::values::Day;
use super::FieldEdited;
use crate::ranges::RangesEdited;
use crate::sections::{SetStoryText, StoryEdited};
use crate::sink::EditSink;
use crate::text::atoms;
use crate::work::Work;
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};

/// What separates the text from "Page X of Y".
const SEPARATOR: &str = " | ";

/// The page number a story is set with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoryNumber {
    /// The page's number.
    Page,
    /// "Page X of Y".
    PageOfPages,
}

/// A story's text set, then a page number put after it.
#[derive(Clone)]
pub struct SetStoryNumbered<'l> {
    /// The text set.
    pub text: SetStoryText,
    /// The page number.
    pub number: StoryNumber,
    /// The day a `DATE` shows (an update's).
    pub today: Day,
    /// The layout of the document's versions.
    pub layout: Option<&'l dyn FieldLayout>,
}

impl std::fmt::Debug for SetStoryNumbered<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetStoryNumbered")
            .field("text", &self.text)
            .field("number", &self.number)
            .finish_non_exhaustive()
    }
}

/// What setting a numbered story did: the text's steps, the text typed before the
/// numbers, and the fields.
#[derive(Debug, Clone)]
pub struct StoryNumbered {
    /// The text set.
    pub story: StoryEdited,
    /// "Page  of " typed (its paragraph's id).
    pub typed: Option<RangesEdited>,
    /// The fields inserted, in order.
    pub fields: Vec<FieldEdited>,
}

impl SetStoryNumbered<'_> {
    /// The story's last paragraph's id and its text's length.
    fn last(sink: &EditSink<'_, '_>, story: &str) -> Result<(String, Vec<char>)> {
        let docx = sink.docx()?;
        let part = docx
            .story(story)
            .ok_or_else(|| EditError::Refused(format!("no story {story}")))?;
        let last = part
            .ids
            .paragraphs()
            .iter()
            .last()
            .ok_or_else(|| EditError::Refused(format!("{story} has no paragraph")))?;
        let work = Work::of(&last.element, Vec::new());
        let text = atoms(&work, work.root()).iter().map(|a| a.ch).collect();
        Ok((last.id, text))
    }

    fn field(&self, at: String, kind: &str) -> InsertField<'_> {
        InsertField {
            at,
            instruction: format!(" {kind}  \\* MERGEFORMAT "),
            result: None,
            simple: kind != "PAGE",
            policy: self.text.policy.clone(),
            today: self.today,
            layout: self.layout,
        }
    }

    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<StoryNumbered> {
        let story = self.text.edit(&mut *sink)?;
        let (id, text) = Self::last(sink, &story.story)?;
        let end = text.len();
        let mut fields = Vec::new();
        let mut typed = None;
        match self.number {
            StoryNumber::Page => fields.push(self.field(format!("{id}@{end}"), "PAGE").edit(&mut *sink)?),
            StoryNumber::PageOfPages => {
                let before = if text.last().is_some_and(|c| !c.is_whitespace()) {
                    SEPARATOR
                } else {
                    ""
                };
                typed = Some(crate::ranges::insert_text(
                    &mut *sink,
                    &format!("{id}@{end}"),
                    &format!("{before}Page  of "),
                )?);
                let (id, text) = Self::last(sink, &story.story)?;
                fields.push(
                    self.field(format!("{id}@{}", text.len()), "NUMPAGES")
                        .edit(&mut *sink)?,
                );
                let (id, _) = Self::last(sink, &story.story)?;
                let at = end + before.chars().count() + 5;
                fields.push(self.field(format!("{id}@{at}"), "PAGE").edit(&mut *sink)?);
            }
        }
        Ok(StoryNumbered { story, typed, fields })
    }
}

impl Command<WordDoc> for SetStoryNumbered<'_> {
    type Out = StoryNumbered;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.text.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<StoryNumbered> {
        let mut sink = EditSink::new(tx, self.text.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}
