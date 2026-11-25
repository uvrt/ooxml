//! Owns: [`Session::block_pages`], the pages a Word document's head version draws body
//! blocks on, laid out through the call's layout (the host's page engine; where the host
//! gives none, the session's own engine for the document, [`Session::pages`]) within the
//! call's limit on laying out: what a host measures once a batch has run (docx-agent's
//! `word_format_table` `keep_together` "taller than a page", said after the batch's last op).
//!
//! Must not: lay out but through the context's layout, or change the document.

use crate::context::CallCancel;
use crate::{CallContext, DocId, Session, SessionError};
use ooxml_base::Cancel;
use ooxml_word_edit::{paragraphs_of, EditError, EditLayout, FieldLayout};

/// Where one block asked is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockAt {
    /// On these pages, numbered from 1 as docx-agent and Word number them (the layout port
    /// counts from 0; this answer is the host's, like the session's other page reads).
    Pages(Vec<usize>),
    /// Outside the body (a header's, a note's): no page of the body's to break.
    Outside,
    /// Not known: why the layout cannot say (a block it does not draw, or names no block).
    Unknown(String),
}

/// Each block asked, in order, or why the version was not laid out at all.
pub type BlockPages = Result<Vec<BlockAt>, String>;

impl Session {
    /// The pages the head version of the Word document `id` draws each of `blocks` on (body
    /// blocks by id, as the version names them), laid out by `context`'s layout (else the
    /// session's own engine for `id`) within its limit on laying out; the outer `Err` says
    /// why the version was not laid out (no layout handed nor kept, the limit passed or the call cancelled, a version the layout does not
    /// lay out).
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; [`SessionError::Kind`] for a document that is not Word.
    pub fn block_pages(
        &self,
        id: &DocId,
        blocks: &[String],
        context: &CallContext,
    ) -> Result<BlockPages, SessionError> {
        let document = self.document(id)?;
        let word = document
            .word()
            .ok_or_else(|| SessionError::Kind("block pages are a Word document's".to_owned()))?;
        let Some(layout) = context.layout.clone().or_else(|| self.own_pages(id)) else {
            return Ok(Err("no layout".to_owned()));
        };
        let head = word.head();
        let cancel = CallCancel::new(context, &*self.context.cancel, &*self.context.clock);
        let layout_cancel = cancel.layout();
        let edit = EditLayout {
            layout: &*layout as &dyn FieldLayout,
            stop: layout_cancel
                .as_ref()
                .map_or(&cancel as &dyn Cancel, |l| l as &dyn Cancel),
            limit: layout_cancel.as_ref().map(|l| l.limit().timeout),
        };
        let laid = match edit.lay_out(head.doc().package()) {
            Ok(laid) => laid,
            Err(error @ (EditError::LayoutStopped(_) | EditError::Unsupported(_))) => {
                return Ok(Err(error.to_string()))
            }
            Err(other) => return Err(other.into()),
        };
        let docx = head.doc().docx()?;
        let outside = |id: &str| {
            paragraphs_of(head.doc(), &docx, id)
                .ok()
                .and_then(|found| found.first().map(|(part, _)| *part != docx.main))
                .unwrap_or(false)
        };
        let ids: Vec<&str> = blocks.iter().map(String::as_str).collect();
        Ok(Ok(ids
            .iter()
            .zip(laid.blocks_pages(&ids))
            .map(|(id, found)| match found {
                _ if outside(id) => BlockAt::Outside,
                Ok(pages) => BlockAt::Pages(pages.into_iter().map(|page| page + 1).collect()),
                Err(unknown) => BlockAt::Unknown(unknown.0),
            })
            .collect()))
    }
}
