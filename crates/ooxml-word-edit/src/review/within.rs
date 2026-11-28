//! Owns: the records a `within` holds when it names text (docx-agent's `_revisions_within`
//! for an address with `@` -- a range, a position, or the text a paragraph or cell
//! holds): a record counts by the paragraph it stands in (its nearest `w:p`), one of the
//! range's; a run-level record only where its characters, in the markup view, meet the
//! range -- an empty range, or a record holding no character, meets every one.
//!
//! Must not: resolve a block or story `within` (the caller does), or change anything.

use super::xml::CONTAINERS;
use crate::names::W;
use crate::{EditError, Result};
use ooxml_word_model::address::Address;
use ooxml_word_model::address::{parse_range, scope_paragraphs};
use ooxml_word_model::docx::revisions::Record;
use ooxml_word_model::docx::text::{offset_before, paragraph_text};
use ooxml_word_model::docx::{at_path, Docx, StoryPart};
use ooxml_word_model::view::View;
use ooxml_xml::Element;
use std::collections::BTreeSet;
use std::sync::Arc;

/// The text a `within` names: its part, its paragraphs (index order), and the offsets it
/// starts at in the first and ends at in the last (`None`: that paragraph's end).
struct Text<'d> {
    part: &'d StoryPart,
    paragraphs: Vec<usize>,
    start: usize,
    end: Option<usize>,
}

/// The ids of the records of `records` inside the text `within` names.
///
/// # Errors
/// The address resolver's refusal.
pub(super) fn records_in_text(docx: &Docx, records: Vec<Record>, within: &str) -> Result<BTreeSet<String>> {
    let text = text_of(docx, within)?;
    let mut found = BTreeSet::new();
    for r in records.into_iter().filter(|r| r.part == text.part.part) {
        if inside(&text, &r) {
            found.insert(r.id);
        }
    }
    Ok(found)
}

/// What `within` names as text (docx-agent's `parse_range`, or `_text_of`'s whole text of
/// the paragraphs it covers).
fn text_of<'d>(docx: &'d Docx, within: &str) -> Result<Text<'d>> {
    let (part, paragraphs) = scope_paragraphs(docx, &Address::parse(within)).map_err(EditError::Address)?;
    let (start, end) = match parse_range(within) {
        Some(range) => (range.start, Some(range.end)),
        None => (0, None),
    };
    Ok(Text {
        part,
        paragraphs,
        start,
        end,
    })
}

/// Whether `record` is inside `text`.
fn inside(text: &Text<'_>, record: &Record) -> bool {
    let root = text.part.root();
    let Some(depth) = (0..record.path.len()).rev().find(|&n| {
        record
            .path
            .get(..n)
            .and_then(|p| at_path(root, p))
            .is_some_and(|e| is_w(e, "p"))
    }) else {
        return false;
    };
    let (held_by, below) = record.path.split_at(depth);
    let Some(paragraph) = text.part.ids.paragraph_at(held_by) else {
        return false;
    };
    let Some(k) = text.paragraphs.iter().position(|&p| p == paragraph.index) else {
        return false;
    };
    if !is_run_record(root, &record.path) {
        return true;
    }
    let element = &paragraph.element;
    let length = || paragraph_text(element, View::Markup).chars().count();
    let start = if k == 0 { text.start } else { 0 };
    let end = if k + 1 == text.paragraphs.len() {
        text.end.unwrap_or_else(length)
    } else {
        length()
    };
    if start == end {
        return true;
    }
    let first = offset_before(element, below, View::Markup);
    let mut next = below.to_vec();
    if let Some(last) = next.last_mut() {
        *last += 1;
    }
    let after = offset_before(element, &next, View::Markup);
    after == first || (first < end && after > start)
}

/// Whether the record at `path` is a content container around runs (docx-agent's
/// `_kind_of` "run": not a paragraph mark's, row's, run mark's or numbering's).
fn is_run_record(root: &Arc<Element>, path: &[u32]) -> bool {
    let Some(element) = at_path(root, path) else {
        return false;
    };
    if !CONTAINERS.iter().any(|c| is_w(element, c)) {
        return false;
    }
    let up = |n: usize| {
        path.len()
            .checked_sub(n)
            .and_then(|k| path.get(..k))
            .and_then(|p| at_path(root, p))
    };
    let parent = up(1);
    let named = |e: Option<&Arc<Element>>, local: &str| e.is_some_and(|e| is_w(e, local));
    !(named(parent, "rPr") || named(parent, "trPr") || named(parent, "numPr"))
}

fn is_w(element: &Element, local: &str) -> bool {
    element.name().namespace() == Some(W) && element.name().local() == local
}
