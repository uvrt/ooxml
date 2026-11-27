//! Owns: what a Word edit's refusal rests on, read in its transaction
//! ([`ooxml_store::Command::read_refusal`], design phase 3 §4: everything an op's outcome
//! depends on, applied or refused, is in the transaction's read set): the block its address
//! names on the working version, by key, as the edit reads it where it applies (a
//! paragraph, a position or range in one, a block an insertion is placed by, a table), and
//! the styles part where it names a style. Where the address names nothing there (a block
//! deleted may come back by an undo), or a range runs across paragraphs, the whole
//! document.
//!
//! Must not: change anything, or name less than an edit's refusal reads: an edit whose
//! refusal rests on more than its target (a block deletion's neighbours, a find's matches)
//! keeps the store's default, the whole document.

use crate::doc::{Key, WordDoc};
use crate::set_text::block_key;
use ooxml_store::Txn;
use ooxml_word_model::address::Address;

/// The paragraph `target` names (an id, an alias, a cell's first paragraph).
pub(crate) fn read_paragraph(tx: &mut Txn<'_, WordDoc>, target: &str) {
    let found = tx
        .doc()
        .docx()
        .and_then(|docx| crate::set_text::find(tx.doc(), &docx, target));
    match found {
        Ok(found) => tx.read(block_key(&found.part, &found.id)),
        Err(_) => tx.read_everything(),
    }
}

/// The paragraph a position or range within one names (`p:A@3`, `p:A@4:11`); across
/// paragraphs, the whole document.
pub(crate) fn read_range(tx: &mut Txn<'_, WordDoc>, range: &Address, position: bool) {
    let found = tx
        .doc()
        .docx()
        .and_then(|docx| crate::ranges::range_of(tx.doc(), &docx, range, position));
    match found {
        Ok((found, None, _, _)) => tx.read(block_key(&found.part, &found.id)),
        _ => tx.read_everything(),
    }
}

/// The paragraph or table `target` names, as an insertion is placed by it.
pub(crate) fn read_anchor(tx: &mut Txn<'_, WordDoc>, target: &str) {
    let found = tx
        .doc()
        .docx()
        .and_then(|docx| crate::paragraphs::anchor(tx.doc(), &docx, target));
    match found {
        Ok(found) => tx.read(block_key(&found.part, &found.id)),
        Err(_) => tx.read_everything(),
    }
}

/// The table `target` names.
pub(crate) fn read_table(tx: &mut Txn<'_, WordDoc>, target: &str) {
    let found = tx
        .doc()
        .docx()
        .and_then(|docx| crate::tables::find_table(tx.doc(), &docx, target));
    match found {
        Ok(found) => tx.read(block_key(&found.part, &found.id)),
        Err(_) => tx.read_everything(),
    }
}

/// The styles part (whether a style named is there, and of which kind); in a document
/// without one (a writer may put it), the whole document.
pub(crate) fn read_styles(tx: &mut Txn<'_, WordDoc>) {
    let styles = tx.doc().docx().ok().and_then(|docx| {
        ooxml_word_model::parts::related(tx.doc().package(), &docx.main, crate::styles::REL_STYLES)
            .ok()
            .flatten()
    });
    match styles {
        Some(part) => tx.read(Key::part(part.as_str())),
        None => tx.read_everything(),
    }
}
