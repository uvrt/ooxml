//! Owns: [`EditCommentText`], comments' text replaced as docx-agent's `edit_comment`
//! replaces it: each comment's paragraphs written anew in the comment styles (added as
//! Word writes them when the document lacks them), the first opening with the annotation
//! mark, the last keeping the paraId the modern comment parts key it by; several at once,
//! one history step.
//!
//! Must not: write the comments part but through the command's copy.

use super::edits::CommentEdited;
use crate::doc::WordDoc;
use crate::names::{W, W14};
use crate::policy::RevisionPolicy;
use crate::review::space::Space;
use crate::review::xml::make;
use crate::sink::EditSink;
use crate::text::write::{check_text, make_run};
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_base::pyfmt::repr_str;
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::AddressError;
use ooxml_word_model::docx::comments::comments;

/// Comments' text replaced, in order: `(comment, text)`, the text a line per paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditCommentText {
    /// The comments and their new text.
    pub edits: Vec<(String, String)>,
}

impl Command<WordDoc> for EditCommentText {
    type Out = Vec<CommentEdited>;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let listed = comments(&*doc.docx()?);
        for (id, text) in &self.edits {
            if !listed.iter().any(|c| c.id == *id) {
                return Err(no_comment(id, &listed));
            }
            check_text(&text.replace('\n', " "))?;
        }
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Vec<CommentEdited>> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        sink.read_everything();
        let mut out = Vec::new();
        for (id, text) in &self.edits {
            check_text(&text.replace('\n', " "))?;
            let docx = sink.docx()?;
            let listed = comments(&docx);
            let comment = listed
                .iter()
                .find(|c| c.id == *id)
                .ok_or_else(|| no_comment(id, &listed))?;
            let style = crate::builtin::ensure(&mut sink, "annotation text", "paragraph")?;
            let reference = crate::builtin::ensure(&mut sink, "annotation reference", "character")?;
            let held = sink.doc().clone();
            let mut space = Space::default();
            space.work(&held, &comment.part)?;
            let node = space
                .locate(&comment.part, &comment.path)
                .ok_or_else(|| EditError::no_element(comment.part.clone()))?;
            let work = space
                .get_mut(&comment.part)
                .ok_or_else(|| EditError::no_part(comment.part.clone()))?;
            for paragraph in work.children(node).to_vec() {
                if work.is(paragraph, W, "p") {
                    work.remove(paragraph);
                }
            }
            let seed = format!("{}\0comment", comment.part);
            let lines: Vec<&str> = text.split('\n').collect();
            for (k, line) in lines.iter().enumerate() {
                let drawn = sink.draw(&seed, 2)?;
                let (Some(para), Some(text_id)) = (drawn.first(), drawn.get(1)) else {
                    return Err(EditError::no_element(comment.part.clone()));
                };
                let para = if k + 1 == lines.len() {
                    comment.last_para.clone()
                } else {
                    para.clone()
                };
                let work = space
                    .get_mut(&comment.part)
                    .ok_or_else(|| EditError::no_part(comment.part.clone()))?;
                let paragraph = comment_paragraph(work, node, (&para, text_id), (&style, &reference), (k == 0, line));
                work.append(node, paragraph);
            }
            space.mark_dirty(&comment.part);
            space.finish(&mut sink)?;
            out.push(CommentEdited {
                id: Some(id.clone()),
                changed: true,
                removed: Vec::new(),
            });
        }
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

/// A comment's paragraph as docx-agent writes it (`_comment_paragraphs`).
pub(super) fn comment_paragraph(
    work: &mut Work,
    holder: Id,
    (para, text): (&str, &str),
    (style, reference): (&str, &str),
    (first, line): (bool, &str),
) -> Id {
    let paragraph = make(work, "p", &[]);
    // lxml, where nothing declares w14, declares it on the paragraph under a made-up prefix
    let prefix = if work.has_prefix_for(holder, W14) {
        None
    } else {
        work.declare(paragraph, "ns1", W14);
        Some("ns1")
    };
    for (local, value) in [("paraId", para), ("textId", text)] {
        let name = match prefix {
            None => work.qname(W14, local, "w14"),
            Some(p) => std::sync::Arc::new(ooxml_xml::QName::new(Some(p), local, Some(W14))),
        };
        work.set(paragraph, name, value);
    }
    let properties = make(work, "pPr", &[]);
    let named = make(work, "pStyle", &[("val", style)]);
    work.append(properties, named);
    work.append(paragraph, properties);
    if first {
        let mark = make(work, "r", &[]);
        let run_properties = make(work, "rPr", &[]);
        let styled = make(work, "rStyle", &[("val", reference)]);
        work.append(run_properties, styled);
        work.append(mark, run_properties);
        let annotation = make(work, "annotationRef", &[]);
        work.append(mark, annotation);
        work.append(paragraph, mark);
    }
    if !line.is_empty() {
        let run = make_run(work, line, None);
        work.append(paragraph, run);
    }
    paragraph
}

fn no_comment(id: &str, listed: &[ooxml_word_model::docx::comments::Comment]) -> EditError {
    EditError::Address(AddressError::not_found(
        format!("no comment {}", repr_str(id)),
        listed.iter().map(|c| c.id.clone()).take(50).collect(),
    ))
}
