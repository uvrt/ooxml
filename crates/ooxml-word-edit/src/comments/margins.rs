//! Owns: no comment written into a header or footer. Word drops every comment anchored in a
//! header or footer when it saves the document (`suite-word.json`, trial
//! `w2-review-comments`: the input's comment in its header and the reply to it both gone
//! after Word's re-save, every other comment kept), so a comment added there, or a reply to
//! a comment anchored there (Word anchors a reply beside its thread's), is refused, the body
//! named as the way out. Comments already in a header or footer are read as they are.
//!
//! Must not: write anything.

use super::reply::holds;
use crate::names::W;
use crate::{EditError, Result};
use ooxml_word_model::docx::{Docx, StoryPart};

/// What a comment in a header or footer comes to.
const DROPPED: &str = "Word drops a comment anchored in a header or footer when it saves the document";

/// Refuse a new comment anchored in `part` where that is a header or footer.
///
/// # Errors
/// `refused`, with the body as the way out.
pub(super) fn refuse_add(docx: &Docx, part: &str) -> Result<()> {
    match docx.part(part).filter(|story| in_margin(story)) {
        Some(story) => Err(EditError::Refused(format!(
            "the place is in {}: {DROPPED}; anchor the comment on text in the body instead",
            story.story
        ))),
        None => Ok(()),
    }
}

/// Refuse a reply to `comment` (the thread's first comment, its `w:id` `parent`) where that
/// comment is anchored in a header or footer.
///
/// # Errors
/// `refused`, with a new comment on the body as the way out.
pub(super) fn refuse_reply(docx: &Docx, comment: &str, parent: &str) -> Result<()> {
    let marked = docx
        .parts
        .iter()
        .find(|story| in_margin(story) && holds(story.root(), parent));
    match marked {
        Some(story) => Err(EditError::Refused(format!(
            "{comment} is anchored in {}, where a reply is anchored too: {DROPPED}; add a comment on \
             text in the body instead",
            story.story
        ))),
        None => Ok(()),
    }
}

/// Whether `story` is a header or a footer (by its root, `w:hdr` or `w:ftr`).
fn in_margin(story: &StoryPart) -> bool {
    let name = story.root().name();
    name.namespace() == Some(W) && matches!(name.local(), "hdr" | "ftr")
}
