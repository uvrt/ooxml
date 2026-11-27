//! Owns: [`AddComments`], comments added and replied to as docx-agent's `add_comment` and
//! `reply_to_comment` write them, in order, one history step: the document stamped first
//! (its renames reported for an added comment), the comment numbered one above the largest
//! annotation id in use, anchored ([`super::anchors`], [`super::reply`]) and written with
//! its entries in the modern parts ([`super::write`]), its author listed in the people
//! part; a reply that also resolves its thread marks it so (`resolve_comment`). Neither is
//! written into a header or footer, where Word drops comments on saving ([`super::margins`]).
//! An item refused where it is planned (before anything is written) is reported as that
//! item's refusal and the others are written (tool rule 5); with none written the call is
//! refused as its first item was.
//!
//! Comments are not revisions: they are written the same with tracking on or off.
//!
//! A comment added on a place named by id reads the largest annotation id in use (its id
//! is one above it, and the comment part holds it as numbered on the edit's version) and
//! writes the blocks it marks by key, the comment parts whole; a reply, or a place named by
//! condition, reads the whole document.
//!
//! Must not: write a comment part but through the edit's copies.

use super::anchors::{anchor, mark, Anchor};
use super::edits::{find, last_para_as_written, set_done, thread_root, CommentEdited};
use super::reply::{has_reference, mark_reply};
use super::size::Sizes;
pub use super::write::Who;
use super::write::{complete, write_comment};
use crate::address_keys::names_by_id;
use crate::doc::WordDoc;
use crate::policy::RevisionPolicy;
use crate::review::space::Space;
use crate::sink::EditSink;
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
    /// The thread resolved after a reply, when asked.
    pub resolved: Option<CommentEdited>,
}

impl Command<WordDoc> for AddComments {
    type Out = Vec<ItemMade>;
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<()> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Vec<ItemMade>> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        self.reads(&mut sink)?;
        let mut out = Vec::with_capacity(self.items.len());
        for item in &self.items {
            // an item refused where it is planned (nothing written yet) is that item's
            // refusal; the others are written (tool rule 5)
            let made = match item {
                CommentWrite::Add { place, text } => match placed(&sink, place) {
                    Ok(anchored) => Ok(self.add(&mut sink, &anchored, text)?),
                    Err(refused) => Err(item_refusal(refused)?),
                },
                CommentWrite::Reply { comment, text, resolve } => match thread(&sink, comment) {
                    Ok(root) => Ok(self.reply(&mut sink, (comment, &root), text, *resolve)?),
                    Err(refused) => Err(item_refusal(refused)?),
                },
            };
            out.push(made);
        }
        if !out.iter().any(Result::is_ok) {
            // nothing to write: the call is refused, as the first item was
            if let Some(Err(first)) = out.into_iter().next() {
                return Err(first);
            }
            return Ok(Vec::new());
        }
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl AddComments {
    /// What the comments depend on besides what they write (the blocks they mark, the
    /// comment parts): added on places named by id, the largest annotation id in use -- a
    /// commit changing it meanwhile is a conflict, as the comment part, put whole, holds
    /// the id as numbered here; a reply (its marks found beside its thread's) or a place
    /// named by condition, the whole document.
    fn reads(&self, sink: &mut EditSink<'_, '_>) -> Result<()> {
        let by_id = self
            .items
            .iter()
            .all(|item| matches!(item, CommentWrite::Add { place, .. } if names_by_id(place)));
        if by_id {
            sink.read_annotation_ceiling()
        } else {
            sink.read_everything();
            Ok(())
        }
    }

    /// A new thread on `anchored`.
    fn add(&self, sink: &mut EditSink<'_, '_>, anchored: &Anchor, text: &str) -> Result<CommentMade> {
        let (id, _) = sink.marker_id()?;
        let docx = sink.docx()?;
        let reference = crate::builtin::ensure(sink, "annotation reference", "character")?;
        let mut space = Space::default();
        let doc = sink.doc().clone();
        mark(&mut space, &doc, anchored, &Sizes::of(&docx), (id, &reference))?;
        let made = self.write(sink, space, (id, text), None)?;
        Ok(CommentMade {
            id: made,
            resolved: None,
        })
    }

    /// A reply in `comment`'s thread (its first comment `root_id`), the thread then
    /// resolved when asked.
    fn reply(
        &self,
        sink: &mut EditSink<'_, '_>,
        (comment, root_id): (&str, &str),
        text: &str,
        resolve: bool,
    ) -> Result<CommentMade> {
        let docx = sink.docx()?;
        let listed = comments(&docx);
        let root = find(&listed, root_id)?;
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

/// One item's outcome: the comment made, or why the item was not written.
pub type ItemMade = std::result::Result<CommentMade, EditError>;

/// Where a new comment on `place` is anchored; refused where it is not a place for one.
///
/// # Errors
/// The place's refusal; a header or footer ([`super::margins`]).
fn placed(sink: &EditSink<'_, '_>, place: &str) -> Result<Anchor> {
    let docx = sink.docx()?;
    let anchored = anchor(sink.doc(), &docx, place)?;
    super::margins::refuse_add(&docx, anchored.part())?;
    Ok(anchored)
}

/// The first comment of `comment`'s thread, which a reply goes beside; refused where a
/// reply cannot go.
///
/// # Errors
/// No such comment; a thread in a header or footer ([`super::margins`]).
fn thread(sink: &EditSink<'_, '_>, comment: &str) -> Result<String> {
    let docx = sink.docx()?;
    let listed = comments(&docx);
    let root = thread_root(&listed, find(&listed, comment)?);
    if let Some(parent) = root.w_id.as_deref() {
        super::margins::refuse_reply(&docx, comment, parent)?;
    }
    Ok(root.id.clone())
}

/// `error` as one item's refusal, where it is one (the call's arguments or the document
/// refusing them); any other error fails the whole call.
///
/// # Errors
/// `error` itself where it is not an item's refusal.
fn item_refusal(error: EditError) -> Result<EditError> {
    match error {
        EditError::Refused(_)
        | EditError::Address(_)
        | EditError::Argument { .. }
        | EditError::Text(_)
        | EditError::Span(_)
        | EditError::Position(_)
        | EditError::Unsupported(_) => Ok(error),
        other => Err(other),
    }
}
