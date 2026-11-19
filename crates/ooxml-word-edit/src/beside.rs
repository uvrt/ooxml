//! Owns: the text edits docx-agent's `word_insert_text` and `word_delete` make at a place
//! named by a range or found as the one place a text occurs (`TextRange.insert_before`,
//! `insert_after`, `delete(collapse_space=)`): [`InsertBeside`] and [`DeletePlaced`], the
//! place resolved on the transaction's own version -- a find reading the paragraphs that hold
//! a match (section 5's read by condition), a range its paragraphs -- so the items of one
//! call, run in turn in one batch, each find the text the ones before left.
//!
//! Must not: write but through the ranges' writer ([`crate::ranges`]), or read outside what
//! it records reading.

use crate::find::{HoldsMatch, Literal, Pattern};
use crate::policy::RevisionPolicy;
use crate::ranges::{replace_place, RangesEdited};
use crate::set_text::{find, Found};
use crate::sink::EditSink;
use crate::text::paragraph_text;
use crate::text_range::{anchor_range, range, range_id, Span};
use crate::work::Work;
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::RangeText;
use ooxml_word_model::docx::Docx;
use std::sync::Arc;

/// Where an edit goes: a range as an address names it (a range id, a position, a
/// paragraph's, cell's, bookmark's or link's text), or the one place a text occurs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextPlace {
    /// An address.
    Range(String),
    /// Text occurring once in the document's stories (`Document.anchor`).
    Once(String),
}

/// Which end of the place new text goes at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// At its start.
    Before,
    /// At its end.
    After,
}

/// Text put at a place's start or end, in the formatting there.
#[derive(Debug, Clone)]
pub struct InsertBeside {
    /// The place.
    pub place: TextPlace,
    /// Which end.
    pub side: Side,
    /// The text (no `\n`).
    pub text: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// A place's text deleted; with `collapse_space`, the one space the deletion would leave
/// doubled or stranded too.
#[derive(Debug, Clone)]
pub struct DeletePlaced {
    /// The place.
    pub place: TextPlace,
    /// Also the space the deletion would leave doubled.
    pub collapse_space: bool,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What a placed edit did: the place's range id and text as found, and the edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// The range found, before the edit (`p:A@4:11`, `p:A@3..p:B@8`).
    pub range: String,
    /// Its text before the edit; paragraph ends are `\n`.
    pub text: String,
    /// The edit.
    pub edited: RangesEdited,
}

/// The place's range on `docx`, recording what it depends on.
fn located(sink: &mut EditSink<'_, '_>, docx: &Docx, place: &TextPlace) -> Result<RangeText> {
    match place {
        TextPlace::Range(address) => range(sink.doc(), docx, address),
        TextPlace::Once(text) => {
            let pattern = Arc::new(Literal::new(text, true, false, "")?);
            let condition: Arc<dyn Pattern> = Arc::clone(&pattern) as Arc<dyn Pattern>;
            // the outcome depends on the paragraphs that hold a match: a write that makes
            // one match, or stop matching, conflicts
            sink.read_where(Arc::new(HoldsMatch::new(condition)));
            anchor_range(sink.doc(), docx, &pattern, text)
        }
    }
}

/// `found` by its paragraphs' ids now (an address written before the stamping as the id
/// it has).
fn current(found: &RangeText, first: &Found, last: &Found) -> RangeText {
    RangeText {
        start_id: first.id.clone(),
        start: found.start,
        end_id: last.id.clone(),
        end: found.end,
    }
}

/// The range's text and its first and last paragraphs.
fn spanned(doc: &WordDoc, docx: &Docx, found: &RangeText) -> Result<(String, Found, Found)> {
    let span = Span::of(doc, docx, found)?;
    let first = find(doc, docx, &found.start_id)?;
    let last = find(doc, docx, &found.end_id)?;
    Ok((span.text(), first, last))
}

impl Command<WordDoc> for InsertBeside {
    type Out = Placed;
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<()> {
        crate::text::write::check_text(&self.text)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Placed> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let docx = sink.docx()?;
        let found = located(&mut sink, &docx, &self.place)?;
        let (text, first, last) = spanned(sink.doc(), &docx, &found)?;
        let now = current(&found, &first, &last);
        let at = match self.side {
            Side::Before => (first, None, found.start, found.start),
            Side::After => (last, None, found.end, found.end),
        };
        let edited = replace_place(&mut sink, &docx, at, &self.text)?;
        Ok(Placed {
            range: range_id(&now),
            text,
            edited,
        })
    }
}

impl Command<WordDoc> for DeletePlaced {
    type Out = Placed;
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<()> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Placed> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let docx = sink.docx()?;
        let found = located(&mut sink, &docx, &self.place)?;
        let (text, first, last) = spanned(sink.doc(), &docx, &found)?;
        let now = current(&found, &first, &last);
        let single = found.start_id == found.end_id || first.path == last.path;
        let (start, end) = if self.collapse_space && single {
            collapsed_space(&first, found.start, found.end)
        } else {
            (found.start, found.end)
        };
        let last = (!single).then_some(last);
        let edited = replace_place(&mut sink, &docx, (first, last, start, end), "")?;
        Ok(Placed {
            range: range_id(&now),
            text,
            edited,
        })
    }
}

/// `TextRange._collapsed_space`: the range `start..end` of one paragraph grown by the one
/// space its deletion would leave doubled or stranded -- the one after it when spaces are
/// on both sides (or it starts the paragraph), the one before it at the paragraph's end or
/// before closing punctuation; as it is when it holds its own space or is empty.
fn collapsed_space(paragraph: &Found, start: usize, end: usize) -> (usize, usize) {
    if start >= end {
        return (start, end);
    }
    let work = Work::of(&paragraph.element, Vec::new());
    let text: Vec<char> = paragraph_text(&work, work.root()).chars().collect();
    let Some(inside) = text.get(start..end) else {
        return (start, end);
    };
    let inside: String = inside.iter().collect();
    if inside.trim_matches(crate::text::diff::python_space) != inside {
        return (start, end);
    }
    let before = start.checked_sub(1).and_then(|k| text.get(k)).copied();
    let after = text.get(end).copied();
    match (before, after) {
        (None | Some(' '), Some(' ')) => (start, end + 1),
        (Some(' '), None | Some('.' | ',' | ';' | ':' | '!' | '?' | ')')) => (start - 1, end),
        _ => (start, end),
    }
}
