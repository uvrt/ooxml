//! Owns: places marked in the text (docx-agent's `_bookmark_markers`, `bookmark`,
//! `_marker_place`, `_marker_range`): a bookmark by name, its range, and the range between
//! any two markers (a comment's) in a view, with its text.
//!
//! [`Bookmarks`] holds every bookmark by name, for a caller that looks many up in one
//! version (a field update's `REF`s and `PAGEREF`s): one walk, not one per name; the walk
//! made of each block's markers ([`BlockMarks`]), which a caller may keep for a block while it
//! is unchanged ([`bookmark_markers_with`]).
//!
//! Must not: resolve block ids (the resolver does) or change the document.

use super::syntax::RangeText;
use super::AddressError;
use crate::docx::text::{offset_before, paragraph_text, View};
use crate::docx::{Docx, StoryPart};
use crate::ids::{Entry, W};
use ooxml_base::pyfmt::repr_str;
use ooxml_xml::{Element, Node};
use std::collections::{HashMap, HashSet};

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

/// The bookmark markers of one block (a part root's child, or the body's): each start's name,
/// id and place, each end's id and place, in document order, places from the block (the
/// block itself at `[]`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockMarks {
    starts: Vec<(String, String, Box<[u32]>)>,
    ends: Vec<(String, Box<[u32]>)>,
}

impl BlockMarks {
    /// The markers of `block` and of every element below it.
    #[must_use]
    pub fn of(block: &Element) -> BlockMarks {
        let mut out = BlockMarks::default();
        let mut visit = |e: &Element, path: &[u32]| {
            if e.name().is(Some(W), "bookmarkStart") {
                out.starts.push((attr(e, "name"), attr(e, "id"), path.into()));
            } else if e.name().is(Some(W), "bookmarkEnd") {
                out.ends.push((attr(e, "id"), path.into()));
            }
        };
        visit(block, &[]);
        every(block, &mut Vec::new(), &mut visit);
        out
    }

    /// Its size in bytes, roughly (what a memo of it weighs).
    #[must_use]
    pub fn weight(&self) -> usize {
        let path = |p: &[u32]| 16 + 4 * p.len();
        64 + self
            .starts
            .iter()
            .map(|(name, id, p)| 48 + name.len() + id.len() + path(p))
            .sum::<usize>()
            + self.ends.iter().map(|(id, p)| 24 + id.len() + path(p)).sum::<usize>()
    }
}

/// Every bookmark's markers, name to (part, start, end), the first start of a name winning,
/// in story and document order (docx-agent's `_bookmark_markers`).
#[must_use]
pub fn bookmark_markers(docx: &Docx) -> Vec<Bookmark<'_>> {
    bookmark_markers_with(docx, &mut |block| std::sync::Arc::new(BlockMarks::of(block)))
}

/// As [`bookmark_markers`], each block's markers from `marks` (which may keep them while the
/// block is unchanged: green elements are immutable).
pub fn bookmark_markers_with<'d>(
    docx: &'d Docx,
    marks: &mut dyn FnMut(&std::sync::Arc<Element>) -> std::sync::Arc<BlockMarks>,
) -> Vec<Bookmark<'d>> {
    let mut out: Vec<Bookmark<'_>> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for part in &docx.parts {
        let mut starts = Vec::new();
        let mut ends: Vec<(&str, Box<[u32]>)> = Vec::new();
        let mut held = Vec::new();
        for (k, node) in part.root().children().iter().enumerate() {
            let Node::Element(child) = node else { continue };
            let k = u32::try_from(k).unwrap_or(u32::MAX);
            if child.name().is(Some(W), "body") {
                for (j, node) in child.children().iter().enumerate() {
                    let Node::Element(block) = node else { continue };
                    held.push((vec![k, u32::try_from(j).unwrap_or(u32::MAX)], marks(block)));
                }
            } else {
                held.push((vec![k], marks(child)));
            }
        }
        let at = |above: &[u32], p: &[u32]| -> Box<[u32]> { [above, p].concat().into_boxed_slice() };
        for (above, block) in &held {
            starts.extend(
                block
                    .starts
                    .iter()
                    .map(|(name, id, p)| (name, id.as_str(), at(above, p))),
            );
            ends.extend(block.ends.iter().map(|(id, p)| (id.as_str(), at(above, p))));
        }
        // The last end of each id, as the walk back from the last one finds it.
        let mut last_end: HashMap<&str, &[u32]> = HashMap::new();
        for (id, path) in &ends {
            last_end.insert(id, path);
        }
        for (name, id, start) in starts {
            if seen.contains(name) {
                continue;
            }
            seen.insert(name.clone());
            let end = last_end.get(id).map(|p| Box::<[u32]>::from(*p));
            out.push(Bookmark {
                name: name.clone(),
                part,
                start,
                end,
            });
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
    Bookmarks::of(docx).get(name).cloned()
}

/// Every bookmark of a version by name, found as [`bookmark`] finds one.
#[derive(Debug, Clone)]
pub struct Bookmarks<'d> {
    markers: Vec<Bookmark<'d>>,
    exact: HashMap<String, usize>,
    folded: HashMap<String, usize>,
}

impl<'d> Bookmarks<'d> {
    /// `docx`'s bookmarks.
    #[must_use]
    pub fn of(docx: &'d Docx) -> Bookmarks<'d> {
        Bookmarks::of_markers(bookmark_markers(docx))
    }

    /// The bookmarks `markers` (as [`bookmark_markers`] finds them) are.
    #[must_use]
    pub fn of_markers(markers: Vec<Bookmark<'d>>) -> Bookmarks<'d> {
        let mut exact = HashMap::new();
        let mut folded = HashMap::new();
        for (k, marker) in markers.iter().enumerate() {
            exact.entry(marker.name.clone()).or_insert(k);
            folded.entry(crate::docx::casefold(&marker.name)).or_insert(k);
        }
        Bookmarks { markers, exact, folded }
    }

    /// The bookmark `name` names (`bm:` optional; matched in any case where no name is
    /// equal).
    ///
    /// # Errors
    /// `not_found` naming it.
    pub fn get(&self, name: &str) -> Result<&Bookmark<'d>, AddressError> {
        let name = name.strip_prefix("bm:").unwrap_or(name);
        self.exact
            .get(name)
            .or_else(|| self.folded.get(&crate::docx::casefold(name)))
            .and_then(|&k| self.markers.get(k))
            .ok_or_else(|| no_bookmark(name))
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
