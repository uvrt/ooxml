//! Owns: places marked in the text (docx-agent's `_bookmark_markers`, `bookmark`,
//! `_marker_place`, `_marker_range`): a bookmark by name, its range, and the range between
//! any two markers (a comment's) in a view, with its text.
//!
//! Must not: resolve block ids (the resolver does) or change the document.

use super::syntax::RangeText;
use super::AddressError;
use crate::docx::text::{offset_before, paragraph_text, View};
use crate::docx::{Docx, StoryPart};
use crate::ids::{Entry, W};
use ooxml_base::pyfmt::repr_str;
use ooxml_xml::{Element, Node};

/// A bookmark: its name as written, its part, and its markers' places there.
#[derive(Debug, Clone)]
pub struct Bookmark<'d> {
    /// Its name.
    pub name: String,
    /// Its part.
    pub part: &'d StoryPart,
    /// Its `w:bookmarkStart`'s place.
    pub start: Box<[u32]>,
    /// Its `w:bookmarkEnd`'s place, where it has one.
    pub end: Option<Box<[u32]>>,
}

/// Every bookmark's markers, name to (part, start, end), the first start of a name winning,
/// in story and document order (docx-agent's `_bookmark_markers`).
#[must_use]
pub fn bookmark_markers(docx: &Docx) -> Vec<Bookmark<'_>> {
    let mut out: Vec<Bookmark<'_>> = Vec::new();
    for part in &docx.parts {
        let mut starts = Vec::new();
        let mut ends: Vec<(String, Box<[u32]>)> = Vec::new();
        every(part.root(), &mut Vec::new(), &mut |e, path| {
            if e.name().is(Some(W), "bookmarkStart") {
                starts.push((attr(e, "name"), attr(e, "id"), path.to_vec().into_boxed_slice()));
            } else if e.name().is(Some(W), "bookmarkEnd") {
                ends.push((attr(e, "id"), path.to_vec().into_boxed_slice()));
            }
        });
        for (name, id, start) in starts {
            if out.iter().any(|b| b.name == name) {
                continue;
            }
            let end = ends.iter().rev().find(|(e, _)| *e == id).map(|(_, p)| p.clone());
            out.push(Bookmark { name, part, start, end });
        }
    }
    out
}

fn attr(e: &Element, local: &str) -> String {
    e.attribute(Some(W), local).unwrap_or("").to_owned()
}

/// Every element below `root` (fallbacks included, as lxml's `iter` walks), with its place.
fn every(root: &Element, path: &mut Vec<u32>, visit: &mut dyn FnMut(&Element, &[u32])) {
    for (k, node) in root.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        visit(child, path);
        every(child, path, visit);
        path.pop();
    }
}

/// The bookmark `name` names (`bm:` optional; matched in any case where no name is equal).
///
/// # Errors
/// `not_found` naming it.
pub fn bookmark<'d>(docx: &'d Docx, name: &str) -> Result<Bookmark<'d>, AddressError> {
    let name = name.strip_prefix("bm:").unwrap_or(name);
    let markers = bookmark_markers(docx);
    let wanted = crate::docx::casefold(name);
    let position = markers
        .iter()
        .position(|b| b.name == name)
        .or_else(|| markers.iter().position(|b| crate::docx::casefold(&b.name) == wanted));
    match position {
        Some(k) => Ok(markers.into_iter().nth(k).ok_or_else(|| no_bookmark(name))?),
        None => Err(no_bookmark(name)),
    }
}

fn no_bookmark(name: &str) -> AddressError {
    AddressError::not_found(format!("no bookmark {}", repr_str(name)), Vec::new())
}

/// The text a bookmark marks, as a range (docx-agent's `_bookmark_range`).
///
/// # Errors
/// `not_found` where its start stands in no indexed paragraph or its story has none.
pub fn bookmark_range(docx: &Docx, bookmark: &Bookmark<'_>) -> Result<RangeText, AddressError> {
    let part = bookmark.part;
    let _ = docx;
    marker_range(part, &bookmark.start, bookmark.end.as_deref(), View::Current)
}

/// The range from the marker at `start` through the one at `end` (or `start` alone) of
/// `part`, in `view` (docx-agent's `_marker_range`).
///
/// # Errors
/// `not_found` where the part has no paragraph.
pub fn marker_range(
    part: &StoryPart,
    start: &[u32],
    end: Option<&[u32]>,
    view: View,
) -> Result<RangeText, AddressError> {
    let (first, first_offset) = marker_place(part, start, true, view)?;
    let (mut last, mut last_offset) = match end {
        None => (first, first_offset),
        Some(end) => marker_place(part, end, false, view)?,
    };
    if (last, last_offset) < (first, first_offset) {
        (last, last_offset) = (first, first_offset);
    }
    let id = |k: usize| part.ids.paragraphs().get(k).map(|p| p.id.clone()).unwrap_or_default();
    Ok(RangeText {
        start_id: id(first),
        start: first_offset,
        end_id: id(last),
        end: last_offset,
    })
}

/// A range's text in `view`; paragraph ends are `\n` (docx-agent's `TextRange.text`).
#[must_use]
pub fn range_text(part: &StoryPart, range: &RangeText, view: View) -> String {
    let index = |id: &str| {
        part.ids.get(id).and_then(|e| match e {
            Entry::Paragraph(k) => Some(k),
            Entry::Table(_) => None,
        })
    };
    let (Some(a), Some(b)) = (index(&range.start_id), index(&range.end_id)) else {
        return String::new();
    };
    let texts: Vec<Vec<char>> = (a..=b)
        .filter_map(|k| part.ids.paragraphs().get(k))
        .map(|p| paragraph_text(&p.element, view).chars().collect())
        .collect();
    let piece = |t: &[char], from: usize, to: usize| -> String {
        t.get(from.min(t.len())..to.min(t.len()).max(from.min(t.len())))
            .unwrap_or(&[])
            .iter()
            .collect()
    };
    match texts.as_slice() {
        [] => String::new(),
        [one] => piece(one, range.start, range.end),
        [first, middle @ .., last] => {
            let mut out = vec![piece(first, range.start, first.len())];
            out.extend(middle.iter().map(|t| t.iter().collect::<String>()));
            out.push(piece(last, 0, range.end));
            out.join("\n")
        }
    }
}

/// The paragraph (its index) and offset a range marker stands at: in its paragraph, or --
/// at block level -- the start of the next paragraph (`forward`) or the end of the one
/// before.
fn marker_place(part: &StoryPart, marker: &[u32], forward: bool, view: View) -> Result<(usize, usize), AddressError> {
    let paragraphs = part.ids.paragraphs();
    let holder = (1..marker.len())
        .rev()
        .find_map(|n| marker.get(..n).and_then(|p| part.ids.paragraph_at(p)));
    if let Some(entry) = holder {
        let below = marker.get(entry.path.len()..).unwrap_or(&[]);
        return Ok((entry.index, offset_before(&entry.element, below, view)));
    }
    let length = |k: usize| {
        paragraphs
            .get(k)
            .map_or(0, |p| paragraph_text(&p.element, view).chars().count())
    };
    let found = if forward {
        part.ids
            .paragraph_after(marker)
            .map(|p| (p.index, 0))
            .or_else(|| paragraphs.last().map(|p| (p.index, length(p.index))))
    } else {
        part.ids
            .paragraph_before(marker)
            .map(|p| (p.index, length(p.index)))
            .or_else(|| paragraphs.first().map(|p| (p.index, 0)))
    };
    found.ok_or_else(|| AddressError::not_found(format!("no paragraph in {}", part.story), Vec::new()))
}
