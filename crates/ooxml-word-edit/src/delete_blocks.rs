//! Owns: [`DeleteBlocks`], `word_delete`'s blocks as docx-agent's tool deletes them: the
//! blocks an address names (one block, a span `p:A..t:B`, a heading's range `h:p:A..`), the
//! outline checked first (round 36, R33: a heading deleted while blocks under it stay is
//! refused -- they would go under the heading before it -- unless forced, which says where
//! they went), then each block deleted in turn as [`crate::DeleteBlock`] deletes it.
//!
//! Must not: write but through the block deletion's writer, or read outside what it records
//! reading: the blocks it deletes, the styles part (a paragraph's level), and the whole
//! document when a heading it deletes leaves blocks under another.

use crate::blocks::{blocks_of, BlockKind};
use crate::delete::BlockDeleted;
use crate::doc::{Key, WordDoc};
use crate::paragraphs::anchor;
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::text::paragraph_text;
use crate::work::Work;
use crate::{EditError, Result};
use ooxml_base::pyfmt::repr_str;
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::outline::{body_blocks, headings, Heading};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::parts::related;
use std::collections::HashSet;
use std::sync::Arc;

/// The styles part's relationship type.
const REL_STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";

/// Delete the blocks an address names, refusing a heading whose content would stay unless
/// `force`.
#[derive(Debug, Clone)]
pub struct DeleteBlocks {
    /// A block (an id, an alias), a span of blocks, or a heading's range.
    pub target: String,
    /// Delete a heading though blocks under it stay.
    pub force: bool,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What the deletion did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlocksDeleted {
    /// The blocks' ids as the address named them, in document order.
    pub ids: Vec<String>,
    /// Each block's deletion, in order.
    pub deleted: Vec<BlockDeleted>,
    /// A forced heading's lines: where the blocks under it now go.
    pub headings: Vec<String>,
}

impl Command<WordDoc> for DeleteBlocks {
    type Out = BlocksDeleted;
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<()> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<BlocksDeleted> {
        let (ids, headings) = {
            let mut sink = EditSink::new(tx, self.policy.clone());
            let docx = sink.docx()?;
            let ids = if self.target.contains("..") {
                block_ids(sink.doc(), &docx, &self.target)?
            } else {
                vec![self.target.clone()]
            };
            let headings = headings_kept(&mut sink, &docx, &ids, self.force)?;
            (ids, headings)
        };
        let mut deleted = Vec::with_capacity(ids.len());
        for id in &ids {
            let mut sink = EditSink::new(tx, self.policy.clone());
            deleted.push(crate::delete_write::delete(&mut sink, id)?);
            crate::textbox::sync(&mut sink)?;
            crate::revision_ids::make_unique(&mut sink)?;
        }
        Ok(BlocksDeleted { ids, deleted, headings })
    }
}

/// The ids of the blocks `address` (a block, a span, a heading's range) names on `doc`,
/// in document order, as [`DeleteBlocks`] reads them.
///
/// # Errors
/// The address's refusal.
pub fn blocks_named(doc: &WordDoc, address: &str) -> Result<Vec<String>> {
    let docx = doc.docx()?;
    block_ids(doc, &docx, address)
}

/// The ids of the blocks `address` names, in document order (docx-agent's `_blocks_of`
/// and `_block_id`).
fn block_ids(doc: &WordDoc, docx: &Docx, address: &str) -> Result<Vec<String>> {
    let (part, blocks) = blocks_of(doc, docx, address)?;
    let story = docx.part(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
    let found = controls(docx);
    blocks
        .iter()
        .map(|block| {
            let id = match block.kind {
                BlockKind::Control => found
                    .iter()
                    .find(|c| c.part == part && *c.path == *block.path)
                    .map(|c| c.id.clone()),
                BlockKind::Paragraph | BlockKind::Table => story.ids.at(&block.path).and_then(|e| story.ids.id_of(e)),
                BlockKind::Custom => None,
            };
            id.ok_or_else(|| EditError::Unsupported(format!("a block of {address} without an id")))
        })
        .collect()
}

/// `_headings_kept`: the refusal of deleting a heading whose content stays, with its ways
/// out (the heading's range, or `force`); forced, the lines saying where it went. Blocks
/// it cannot resolve leave the check to the deletion's own refusal.
fn headings_kept(sink: &mut EditSink<'_, '_>, docx: &Docx, ids: &[String], force: bool) -> Result<Vec<String>> {
    let Ok(canonical) = ids
        .iter()
        .map(|i| anchor(sink.doc(), docx, i).map(|a| a.id))
        .collect::<Result<Vec<String>>>()
    else {
        return Ok(Vec::new());
    };
    // a paragraph's level is its own or its style chain's
    if let Some(part) = related(sink.doc().package(), &docx.main, REL_STYLES)? {
        sink.read(Key::Part(Arc::from(part.as_str())));
    }
    let blocks = body_blocks(docx, &controls(docx));
    let found = headings(&blocks);
    let gone: HashSet<&str> = canonical.iter().map(String::as_str).collect();
    if !found.iter().any(|h| gone.contains(h.id.as_str())) {
        return Ok(Vec::new());
    }
    // a heading goes: where its blocks go depends on the whole outline
    sink.read_everything();
    let mut left: Vec<(&Heading, usize, String, Option<&Heading>)> = Vec::new();
    for heading in found.iter().filter(|h| gone.contains(h.id.as_str())) {
        let kept: Vec<&String> = blocks
            .get(heading.start + 1..heading.end)
            .unwrap_or_default()
            .iter()
            .filter_map(|b| b.id.as_ref())
            .filter(|id| !gone.contains(id.as_str()))
            .collect();
        let Some(first) = kept.first() else { continue };
        let under = found
            .iter()
            .rev()
            .find(|h| h.start < heading.start && !gone.contains(h.id.as_str()));
        left.push((heading, kept.len(), (*first).clone(), under));
    }
    if left.is_empty() {
        return Ok(Vec::new());
    }
    let lines: Vec<String> = left
        .iter()
        .map(|(heading, kept, first, under)| {
            format!(
                "{} is a heading: its {kept} block(s) from {first} (to {}) would go under {}",
                label(Some(heading)),
                heading.last,
                label(*under)
            )
        })
        .collect();
    if force {
        return Ok(lines.iter().map(|l| l.replace("would go", "now go")).collect());
    }
    let range = |h: &Heading| format!("h:{}", h.id);
    let mut message = lines.iter().take(5).cloned().collect::<Vec<_>>().join("; ");
    if lines.len() > 5 {
        message.push_str(&format!("; and {} more", lines.len() - 5));
    }
    let ranges: Vec<String> = left.iter().map(|(h, ..)| range(h)).collect();
    message.push_str(&format!(
        ". To delete a heading with everything under it, target its range {}; to delete the heading alone and keep its content, force true",
        ranges.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
    ));
    let mut options: Vec<String> = ranges.into_iter().take(50).collect();
    options.push("force".to_owned());
    Err(EditError::Argument {
        field: "target".to_owned(),
        message,
        options,
    })
}

/// `p:A 'Results'`, or the start of the document for none (docx-agent's `label`).
fn label(heading: Option<&Heading>) -> String {
    let Some(heading) = heading else {
        return "no heading (the document's start)".to_owned();
    };
    let work = Work::of(&heading.element, Vec::new());
    let text = paragraph_text(&work, work.root());
    let words = text
        .split(crate::text::diff::python_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if words.is_empty() {
        return heading.id.clone();
    }
    format!(
        "{} {}",
        heading.id,
        repr_str(&words.chars().take(40).collect::<String>())
    )
}
