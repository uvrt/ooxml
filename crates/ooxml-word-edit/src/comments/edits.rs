//! Owns: [`CommentEdits`], resolving, reopening and deleting comments as docx-agent does
//! (`edit/comments.py`: `_set_done` on a thread's first comment, `w15:done` in
//! `commentsExtended`; `delete_comment` with its replies), several at once, one history
//! step.
//!
//! Must not: write a comment part but through the command's copies (a part it lacks is
//! made through the sink first).

use super::delete::{comment_part_relationships, delete, none_left};
use super::{part_of, REL_EXTENDED, W15};
use crate::doc::WordDoc;
use crate::policy::RevisionPolicy;
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_base::pyfmt::repr_str;
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::AddressError;
use ooxml_word_model::docx::comments::{comments, Comment};
use ooxml_word_model::docx::Docx;
use std::collections::BTreeSet;

/// What to do to each comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentEdit {
    /// Mark its thread resolved.
    Resolve,
    /// Mark its thread open again.
    Reopen,
    /// Delete it with its replies.
    Delete,
}

/// Resolve, reopen or delete comments, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentEdits {
    /// What to do.
    pub edit: CommentEdit,
    /// The comments (`c:<durableId>`, `c#<w:id>`).
    pub ids: Vec<String>,
}

/// What one comment's edit did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommentEdited {
    /// The thread's first comment (resolve, reopen).
    pub id: Option<String>,
    /// Whether it changed anything.
    pub changed: bool,
    /// The comments deleted (delete).
    pub removed: Vec<String>,
}

/// The comment `id`, or the refusal naming the document's.
pub(super) fn find<'c>(listed: &'c [Comment], id: &str) -> Result<&'c Comment> {
    listed.iter().find(|c| c.id == id).ok_or_else(|| {
        EditError::Address(AddressError::not_found(
            format!("no comment {}", repr_str(id)),
            listed.iter().map(|c| c.id.clone()).take(50).collect(),
        ))
    })
}

/// The first comment of `comment`'s thread.
pub(super) fn thread_root<'c>(listed: &'c [Comment], comment: &'c Comment) -> &'c Comment {
    let mut at = comment;
    let mut seen = BTreeSet::new();
    while let Some(parent) = at.parent.as_deref().and_then(|p| listed.iter().find(|c| c.id == p)) {
        if !seen.insert(at.id.clone()) {
            break;
        }
        at = parent;
    }
    at
}

impl Command<WordDoc> for CommentEdits {
    type Out = Vec<CommentEdited>;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let docx = doc.docx()?;
        let listed = comments(&docx);
        for id in &self.ids {
            find(&listed, id)?;
        }
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Vec<CommentEdited>> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        sink.read_everything();
        let docx = sink.docx()?;
        let doc = sink.doc().clone();
        let listed = comments(&docx);
        let mut space = Space::default();
        let mut out = Vec::new();
        let mut gone = BTreeSet::new();
        let mut drop_parts = false;
        for id in &self.ids {
            let left: Vec<Comment> = listed.iter().filter(|c| !gone.contains(&c.id)).cloned().collect();
            // a comment deleted with a thread named before it is no comment any more
            let comment = find(&left, id)?;
            match self.edit {
                CommentEdit::Delete => {
                    let removed = delete(&doc, &docx, &mut space, &listed, &comment.id.clone(), &mut gone)?;
                    drop_parts |= none_left(&doc, &docx, &space)?;
                    out.push(CommentEdited {
                        id: None,
                        changed: true,
                        removed,
                    });
                }
                CommentEdit::Resolve | CommentEdit::Reopen => {
                    let root = thread_root(&left, comment);
                    let changed = set_done(&mut sink, &docx, &mut space, root, self.edit == CommentEdit::Resolve)?;
                    out.push(CommentEdited {
                        id: Some(root.id.clone()),
                        changed,
                        removed: Vec::new(),
                    });
                }
            }
        }
        space.finish(&mut sink)?;
        if drop_parts {
            let rels = comment_part_relationships(&docx);
            sink.release(&docx.main, &rels)?;
        }
        if self.edit == CommentEdit::Delete {
            let authors = crate::review::authors_in_use(&docx);
            for author in &authors {
                sink.people(None, Some(author))?;
            }
        }
        Ok(out)
    }
}

/// `w15:done` of `root`'s entry in `commentsExtended` set (`_set_done`); whether it changed.
pub(super) fn set_done(sink: &mut EditSink<'_, '_>, docx: &Docx, space: &mut Space, root: &Comment, done: bool) -> Result<bool> {
    let para = last_para_as_written(docx, root);
    if para.is_empty() {
        return Err(EditError::Refused(format!(
            "{} has no paragraph id to mark resolved by",
            root.id
        )));
    }
    let part = match part_of(sink.doc().package(), &docx.main, REL_EXTENDED) {
        Some(part) => part,
        None => super::make_part(sink, &docx.main, super::Kind::Extended)?,
    };
    let doc = sink.doc().clone();
    let work = space
        .work(&doc, &part)?
        .ok_or_else(|| EditError::NoPart(part.clone()))?;
    let value = if done { "1" } else { "0" };
    let found = work.children(work.root()).iter().copied().find(|e| {
        work.is(*e, W15, "commentEx")
            && work.get(*e, Some(W15), "paraId").unwrap_or("").to_uppercase() == para.to_uppercase()
    });
    let entry = match found {
        Some(e) => e,
        None => {
            let name = work.qname(W15, "commentEx", "w15");
            let e = work.new_element(name);
            let attr = work.qname(W15, "paraId", "w15");
            work.set(e, attr, &para);
            let root_id = work.root();
            work.append(root_id, e);
            e
        }
    };
    if work.get(entry, Some(W15), "done") == Some(value) {
        return Ok(false);
    }
    let attr = work.qname(W15, "done", "w15");
    work.set(entry, attr, value);
    space.mark_dirty(&part);
    Ok(true)
}

/// The `w14:paraId` of a comment's last paragraph as written (`_last_para_id`).
pub(super) fn last_para_as_written(docx: &Docx, comment: &Comment) -> String {
    let Some(element) = docx
        .part(&comment.part)
        .and_then(|p| ooxml_word_model::docx::at_path(p.root(), &comment.path))
    else {
        return String::new();
    };
    let mut last = String::new();
    let mut stack = vec![element.clone()];
    while let Some(e) = stack.pop() {
        if e.name().is(Some(crate::names::W), "p") {
            last = e.attribute(Some(crate::names::W14), "paraId").unwrap_or("").to_owned();
        }
        stack.extend(e.child_elements().cloned().collect::<Vec<_>>().into_iter().rev());
    }
    last
}
