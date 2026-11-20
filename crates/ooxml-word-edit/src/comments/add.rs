//! Owns: [`AddComments`], comments added and replied to as docx-agent's `add_comment` and
//! `reply_to_comment` write them, in order, one history step: the document stamped first
//! (its renames reported for an added comment), the comment numbered one above the largest
//! annotation id in use, anchored ([`super::anchors`], [`super::reply`]) and written with
//! its entries in the modern parts ([`super::write`]), its author listed in the people
//! part; a reply that also resolves its thread marks it so (`resolve_comment`).
//!
//! Comments are not revisions: they are written the same with tracking on or off.
//!
//! Must not: write a comment part but through the edit's copies; it reads every story
//! (annotation ids are the whole document's).

use super::anchors::{anchor, mark};
use super::edits::{find, last_para_as_written, set_done, thread_root, CommentEdited};
use super::reply::{has_reference, mark_reply};
use super::size::Sizes;
pub use super::write::Who;
use super::write::{complete, write_comment};
use crate::doc::WordDoc;
use crate::policy::RevisionPolicy;
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::stamping::stamp_document;
use crate::{EditError, Result};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::comments::comments;

/// A comment to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentWrite {
    /// A new thread on `place`: a range or position, a paragraph, a cell, a table or a
    /// row.
    Add {
        /// What it comments on.
        place: String,
        /// Its text, a line per paragraph.
        text: String,
    },
    /// A reply in the thread of `comment`.
    Reply {
        /// The comment replied to (its thread's first comment takes the reply).
        comment: String,
        /// Its text, a line per paragraph.
        text: String,
        /// Also mark the thread resolved.
        resolve: bool,
    },
}

/// Comments added and replied to, in order, by one author at one time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddComments {
    /// What to write.
    pub items: Vec<CommentWrite>,
    /// Who writes them, and when.
    pub who: Who,
}

/// What one comment's writing did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommentMade {
    /// The new comment (`c:<durableId>`).
    pub id: String,
    /// Ids the stamping before it renamed, old to new (an added comment's).
    pub renamed: Vec<(String, String)>,
    /// The thread resolved after a reply, when asked.
    pub resolved: Option<CommentEdited>,
}

impl Command<WordDoc> for AddComments {
    type Out = Vec<CommentMade>;
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<()> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Vec<CommentMade>> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        sink.read_everything();
        let mut out = Vec::with_capacity(self.items.len());
        for item in &self.items {
            let made = match item {
                CommentWrite::Add { place, text } => self.add(&mut sink, place, text)?,
                CommentWrite::Reply { comment, text, resolve } => self.reply(&mut sink, comment, text, *resolve)?,
            };
            out.push(made);
        }
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl AddComments {
    /// A new thread on `place`.
    fn add(&self, sink: &mut EditSink<'_, '_>, place: &str, text: &str) -> Result<CommentMade> {
        let docx = sink.docx()?;
        let anchored = anchor(sink.doc(), &docx, place)?;
        let renamed = stamp_document(sink, &docx)?;
        let (id, _) = sink.marker_id()?;
        let docx = sink.docx()?;
        let reference = crate::builtin::ensure(sink, "annotation reference", "character")?;
        let mut space = Space::default();
        let doc = sink.doc().clone();
        mark(&mut space, &doc, &anchored, &Sizes::of(&docx), (id, &reference))?;
        let made = self.write(sink, space, (id, text), None)?;
        Ok(CommentMade {
            id: made,
            renamed,
            resolved: None,
        })
    }

    /// A reply in `comment`'s thread, the thread then resolved when asked.
    fn reply(&self, sink: &mut EditSink<'_, '_>, comment: &str, text: &str, resolve: bool) -> Result<CommentMade> {
        let docx = sink.docx()?;
        let root_id = {
            let listed = comments(&docx);
            thread_root(&listed, find(&listed, comment)?).id.clone()
        };
        stamp_document(sink, &docx)?;
        let docx = sink.docx()?;
        let listed = comments(&docx);
        let root = find(&listed, &root_id)?;
        let (id, _) = sink.marker_id()?;
        let mut space = Space::default();
        if let Some(parent) = root.w_id.as_deref() {
            // the reference style is made where a reference run is written, before the
            // comment's own paragraphs ask for it
            let reference = if has_reference(&docx, parent) {
                crate::builtin::ensure(sink, "annotation reference", "character")?
            } else {
                String::new()
            };
            let doc = sink.doc().clone();
            mark_reply(&mut space, &doc, &docx, parent, (id, &reference))?;
        }
        let parent_para = last_para_as_written(&docx, root);
        let made = self.write(sink, space, (id, text), Some(&parent_para))?;
        let resolved = if resolve {
            let docx = sink.docx()?;
            let listed = comments(&docx);
            let root = thread_root(&listed, find(&listed, comment)?).clone();
            let mut space = Space::default();
            let changed = set_done(sink, &docx, &mut space, &root, true)?;
            space.finish(sink)?;
            Some(CommentEdited {
                id: Some(root.id.clone()),
                changed,
                removed: Vec::new(),
            })
        } else {
            None
        };
        Ok(CommentMade {
            id: made,
            renamed: Vec::new(),
            resolved,
        })
    }

    /// The comment written into the comment parts, its author listed, every comment given
    /// its entries, and `space`'s copies put back; its id.
    fn write(
        &self,
        sink: &mut EditSink<'_, '_>,
        mut space: Space,
        (id, text): (i64, &str),
        parent: Option<&str>,
    ) -> Result<String> {
        let main = sink.docx()?.main.clone();
        let (durable, parts) = write_comment(sink, &mut space, &main, (id, text), &self.who, parent)?;
        sink.people(Some(&self.who.author), None)?;
        complete(sink, &mut space, &main, &parts.comments)?;
        space.finish(sink)?;
        sink.ensure_w14(&parts.comments, false, &[])?;
        Ok(format!("c:{durable}"))
    }
}
