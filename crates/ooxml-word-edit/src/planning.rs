//! Owns: what a tool plans its edit from, read in the edit's transaction on the version the
//! edit is applied to, with what it depends on recorded there: the one place a text occurs
//! ([`anchor_in`], as [`crate::anchor`] finds it), the reads of a find the caller made on the
//! transaction's version itself ([`read_matching`], [`read_ranges`]), a heading's section ([`section_in`]) and
//! the body's last block ([`last_block_in`], what `end` is after). A commit meanwhile that
//! changes what was read is a conflict, and the plan is made again where the edit is
//! prepared again -- never applied as worked out on the version before that commit.
//!
//! Must not: change anything.

use crate::doc::{Key, WordDoc};
use crate::find::{HoldsMatch, Literal, Pattern};
use crate::text_range::{anchor_range, range_id};
use crate::Result;
use ooxml_store::Txn;
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::outline::body_blocks;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::Entry;
use std::sync::Arc;

/// The one place `text` occurs on the transaction's version, as a range id ([`crate::anchor`]:
/// every story's paragraphs, or the paragraphs `within` holds; case-sensitive, not across
/// paragraphs), its reads recorded in `tx` ([`read_matching`], [`read_ranges`]).
///
/// # Errors
/// As [`crate::anchor`]'s.
pub fn anchor_in(tx: &mut Txn<'_, WordDoc>, text: &str, within: Option<&str>) -> Result<String> {
    let docx = tx.doc().docx()?;
    let pattern = Literal::new(text, true, false, "")?;
    // read before the find: a refusal (found nowhere, or more than once) reads it too
    tx.read_where(Arc::new(HoldsMatch::new(Arc::new(pattern.clone()))));
    let found = anchor_range(tx.doc(), &docx, (&pattern, text), within).map(|r| range_id(&r))?;
    read_paragraphs(tx, &docx, [found.as_str()]);
    Ok(found)
}

/// Record in `tx` that a plan read where `text` occurs (case-sensitive, within paragraphs):
/// every paragraph that holds a match where a commit is applied, by condition -- so a write
/// that makes a match is a conflict. With [`read_ranges`] of what it found, a write that
/// takes a found match away or moves it is one too.
///
/// # Errors
/// `nothing to find` for empty text.
pub fn read_matching(tx: &mut Txn<'_, WordDoc>, text: &str) -> Result<()> {
    let pattern: Arc<dyn Pattern> = Arc::new(Literal::new(text, true, false, "")?);
    tx.read_where(Arc::new(HoldsMatch::new(pattern)));
    Ok(())
}

/// Record in `tx` the paragraphs the range ids `ranges` start and end in (`p:A@4:11`,
/// `p:A@3..p:B@8`), by key.
///
/// # Errors
/// When a story part is not XML.
pub fn read_ranges<'a>(tx: &mut Txn<'_, WordDoc>, ranges: impl IntoIterator<Item = &'a str>) -> Result<()> {
    let docx = tx.doc().docx()?;
    read_paragraphs(tx, &docx, ranges);
    Ok(())
}

/// The ids of the heading `heading`'s section on the transaction's version
/// ([`crate::section_blocks`]), recorded in `tx` as read: each block of it by key -- a block
/// put into it is put beside one of them, writing its key --, the block after it (whether it
/// is a heading that ends the section), and the styles part (which styles are headings).
///
/// # Errors
/// As [`crate::section_blocks`]'.
pub fn section_in(tx: &mut Txn<'_, WordDoc>, heading: &str) -> Result<Vec<String>> {
    let ids = crate::section_blocks(tx.doc(), heading)?;
    let docx = tx.doc().docx()?;
    let blocks = body_ids(&docx);
    let after = ids
        .last()
        .and_then(|last| blocks.iter().position(|b| b == last))
        .and_then(|k| blocks.get(k + 1));
    for id in ids.iter().chain(after) {
        tx.read(Key::block(docx.main.as_str(), id.as_str()));
    }
    let styles = ooxml_word_model::parts::related(tx.doc().package(), &docx.main, crate::styles::REL_STYLES);
    if let Ok(Some(styles)) = styles {
        tx.read(Key::part(styles.as_str()));
    }
    Ok(ids)
}

/// The id of the body's last block on the transaction's version (what `end` is after; none
/// in an empty body), recorded in `tx` as read: a block put after it writes its key, so an
/// append meanwhile is a conflict and the edit lands after it, in the order of the commits.
/// An empty body, or a last block without an id, reads the whole part.
///
/// # Errors
/// When the body is not XML.
pub fn last_block_in(tx: &mut Txn<'_, WordDoc>) -> Result<Option<String>> {
    let docx = tx.doc().docx()?;
    let blocks = body_blocks(&docx, &controls(&docx));
    tx.read(match blocks.last().and_then(|b| b.id.as_deref()) {
        Some(id) => Key::block(docx.main.as_str(), id),
        None => Key::part(docx.main.as_str()),
    });
    Ok(blocks.into_iter().filter_map(|b| b.id).next_back())
}

/// The ids of the body's top-level blocks, in order (docx-agent's `blocks()` ids).
fn body_ids(docx: &Docx) -> Vec<String> {
    body_blocks(docx, &controls(docx))
        .into_iter()
        .filter_map(|b| b.id)
        .collect()
}

/// The keys of the paragraphs the range ids `ranges` start and end in (`p:A@4:11`,
/// `p:A@3..p:B@8`), read in `tx`.
fn read_paragraphs<'a>(tx: &mut Txn<'_, WordDoc>, docx: &Docx, ranges: impl IntoIterator<Item = &'a str>) {
    for range in ranges {
        for end in range.split("..") {
            let id = end.split('@').next().unwrap_or(end);
            let part = docx
                .parts
                .iter()
                .find(|story| matches!(story.ids.get(id), Some(Entry::Paragraph(_))));
            if let Some(story) = part {
                tx.read(Key::block(story.part.as_str(), id));
            }
        }
    }
}
