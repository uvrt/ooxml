//! Owns: [`InsertCaption`], a caption paragraph put beside a block as Word's Insert
//! Caption writes one (docx-agent's `insert_caption`): in the Caption style (added as Word
//! writes it), the label and a space, its `SEQ <label> \* ARABIC` number as a simple field
//! counted through the document, then the caption's text in a plain run of its own.
//!
//! Must not: write but through the sink.

use super::insert::InsertField;
use super::pages::EditLayout;
use super::values::Day;
use super::FieldEdited;
use crate::paragraphs::insert_paragraph;
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::text::write::make_run;
use crate::work::{scope_above, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::at_path;

/// A caption inserted.
#[derive(Clone)]
pub struct InsertCaption<'l> {
    /// The block it goes beside.
    pub anchor: String,
    /// After the block (`true`) or before it.
    pub after: bool,
    /// The label (`Figure`, `Table`).
    pub label: String,
    /// The text after its number (`": Results"`).
    pub text: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
    /// The day a `DATE` shows (none here; an update's).
    pub today: Day,
    /// The layout of the document's versions.
    pub layout: Option<EditLayout<'l>>,
}

impl std::fmt::Debug for InsertCaption<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InsertCaption")
            .field("anchor", &self.anchor)
            .field("after", &self.after)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// Whether `label` is a word that is no number at its start (`[^\W\d]\w*`).
pub(crate) fn is_label(label: &str) -> bool {
    let mut chars = label.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || c == '_')
}

impl InsertCaption<'_> {
    fn check(&self, doc: &WordDoc) -> Result<()> {
        if !is_label(&self.label) {
            return Err(EditError::Refused(format!(
                "{} is not a caption label",
                ooxml_base::pyfmt::repr_str(&self.label)
            )));
        }
        let docx = doc.docx()?;
        crate::builtin::resolve(&docx, "caption", "paragraph")?;
        crate::paragraphs::anchor(doc, &docx, &self.anchor).map(|_| ())
    }

    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<FieldEdited> {
        self.check(sink.doc())?;
        let label = format!("{} ", self.label);
        let inserted = insert_paragraph(&mut *sink, &self.anchor, (self.after, &label, Some("caption")))?;
        let length = label.chars().count();
        let field = InsertField {
            at: format!("{}@{length}", inserted.id),
            instruction: format!(" SEQ {} \\* ARABIC ", self.label),
            result: None,
            simple: true,
            policy: self.policy.clone(),
            today: self.today,
            layout: self.layout,
            locale: None, // a SEQ field reads no style names
        };
        let made = field.edit(&mut *sink)?;
        if !self.text.is_empty() {
            self.append_text(sink, &inserted.part, &inserted.id)?;
        }
        let mut created = vec![inserted.id.to_string()];
        created.extend(made.created);
        let mut renamed = inserted.renamed;
        renamed.extend(made.renamed);
        Ok(FieldEdited {
            id: Some(inserted.id.to_string()),
            changed: true,
            created,
            renamed,
            unknown: made.unknown,
            ..FieldEdited::default()
        })
    }

    /// The caption's text in a plain run of its own at the paragraph's end (tracked: an
    /// insertion).
    fn append_text(&self, sink: &mut EditSink<'_, '_>, part: &str, id: &str) -> Result<()> {
        let docx = sink.docx()?;
        let story = docx.part(part).ok_or_else(|| EditError::no_part(part.to_owned()))?;
        let paragraph = story
            .ids
            .paragraphs()
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| EditError::no_element(part.to_owned()))?;
        let path = paragraph.path.to_vec();
        let tree = sink
            .doc()
            .tree(part)?
            .ok_or_else(|| EditError::no_part(part.to_owned()))?;
        let element = at_path(tree.root(), &path).ok_or_else(|| EditError::no_element(part.to_owned()))?;
        let mut work = Work::of(element, scope_above(tree.root(), &path));
        let root = work.root();
        let run = make_run(&mut work, &self.text, None);
        work.append(root, run);
        let tracked = super::insert::track(&mut *sink, &mut work, &[run], false)?;
        let place = super::insert::Place {
            part: part.to_owned(),
            path,
            id: id.to_owned(),
            offset: 0,
        };
        super::insert::put_back(&mut *sink, &work, (&place, false), tracked, &[])
    }
}

impl Command<WordDoc> for InsertCaption<'_> {
    type Out = FieldEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.check(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<FieldEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}
