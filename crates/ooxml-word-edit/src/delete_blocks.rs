//! Owns: [`DeleteBlocks`], `word_delete`'s blocks as docx-agent's tool deletes them: the
//! blocks an address names (one block, a span `p:A..t:B`, a heading's range `h:p:A..`), the
//! outline checked first (round 36, R33: a heading deleted while blocks under it stay is
//! refused -- they would go under the heading before it -- unless forced, which says where
//! they went), then each block deleted in turn as [`crate::DeleteBlock`] deletes it.
//!
//! Must not: write but through the block deletion's writer, or read outside what it records
//! reading: the blocks it deletes, the styles part (a paragraph's level), and the whole
//! document when a heading it deletes leaves blocks under another. A heading it deletes is
//! read with its own range alone, not the whole outline.

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
use ooxml_word_model::address::resolve::resolve;
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::outline::{BodyWalk, Step};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::Entry;
use ooxml_word_model::parts::related;
use ooxml_xml::Element;
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
    let story = docx.part(&part).ok_or_else(|| EditError::no_part(part.clone()))?;
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
/// it cannot resolve leave the check to the deletion's own refusal. Each heading that goes
/// is read from itself to its range's end (the next heading of its level or higher), and
/// back to the heading its blocks would go under; nothing else of the outline is built.
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
    let Some(walk) = BodyWalk::new(docx) else {
        return Ok(Vec::new());
    };
    let mut going: Vec<Step> = canonical.iter().filter_map(|id| heading(docx, &walk, id)).collect();
    if going.is_empty() {
        // no heading goes: the outline decides nothing, and is not read
        return Ok(Vec::new());
    }
    going.sort_by_key(|s| s.child);
    going.dedup_by_key(|s| s.child);
    // a heading goes: where its blocks go depends on the outline around it
    sink.read_everything();
    let gone: HashSet<&str> = canonical.iter().map(String::as_str).collect();
    let left: Vec<Left> = going.into_iter().filter_map(|h| left_of(&walk, h, &gone)).collect();
    if left.is_empty() {
        return Ok(Vec::new());
    }
    let lines: Vec<String> = left
        .iter()
        .map(|l| {
            format!(
                "{} is a heading: its {} block(s) from {} (to {}) would go under {}",
                label(Some(&l.heading)),
                l.kept,
                l.first,
                l.last,
                label(l.under.as_ref())
            )
        })
        .collect();
    if force {
        return Ok(lines.iter().map(|l| l.replace("would go", "now go")).collect());
    }
    let mut message = lines.iter().take(5).cloned().collect::<Vec<_>>().join("; ");
    if lines.len() > 5 {
        message.push_str(&format!("; and {} more", lines.len() - 5));
    }
    let ranges: Vec<String> = left.iter().map(|l| format!("h:{}", l.heading.0)).collect();
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

/// A heading that goes while blocks of its range stay: it (id, paragraph), how many stay,
/// the first and the range's last block, and the heading they would go under.
struct Left {
    heading: (String, Arc<Element>),
    kept: usize,
    first: String,
    last: String,
    under: Option<(String, Arc<Element>)>,
}

/// What of `heading`'s range stays when the blocks `gone` go, read from the heading to its
/// range's end and back to the heading before it that stays; `None` when nothing stays.
fn left_of(walk: &BodyWalk<'_>, heading: Step, gone: &HashSet<&str>) -> Option<Left> {
    let (level, id) = (heading.level?, heading.id?);
    let (mut kept, mut first, mut last) = (0, None, None);
    for step in walk.after(heading.child) {
        if step.level.is_some_and(|l| l <= level) {
            break;
        }
        let Some(block) = step.id else { continue };
        if !gone.contains(block.as_str()) {
            kept += 1;
            first.get_or_insert_with(|| block.clone());
        }
        last = Some(block);
    }
    let under = walk
        .before(heading.child)
        .find(|s| s.level.is_some() && s.id.as_deref().is_some_and(|i| !gone.contains(i)));
    Some(Left {
        first: first?,
        last: last.unwrap_or_else(|| id.clone()),
        heading: (id, heading.element),
        kept,
        under: under.and_then(|s| Some((s.id?, s.element))),
    })
}

/// The block `id` names when it is a heading: a paragraph of the body's top level with an
/// outline level, its own or its style chain's (what `headings` finds), read for it alone.
fn heading(docx: &Docx, walk: &BodyWalk<'_>, id: &str) -> Option<Step> {
    let Ok((part, Entry::Paragraph(k))) = resolve(docx, id) else {
        return None;
    };
    let paragraph = part.ids.paragraphs().get(k)?;
    let top = part.part == docx.main && paragraph.path.len() == 2 && paragraph.path.first() == Some(&walk.body_index());
    let step = walk.at(*paragraph.path.get(1)? as usize).filter(|_| top)?;
    step.level.is_some().then_some(step)
}

/// `p:A 'Results'`, or the start of the document for none (docx-agent's `label`).
fn label(heading: Option<&(String, Arc<Element>)>) -> String {
    let Some((id, element)) = heading else {
        return "no heading (the document's start)".to_owned();
    };
    let work = Work::of(element, Vec::new());
    let text = paragraph_text(&work, work.root());
    let words = text
        .split(crate::text::diff::python_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if words.is_empty() {
        return id.clone();
    }
    format!("{id} {}", repr_str(&words.chars().take(40).collect::<String>()))
}
