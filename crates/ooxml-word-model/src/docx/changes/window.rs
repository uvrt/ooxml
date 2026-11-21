//! Owns: the listed changes holding given records ([`changes_holding`], with their authors
//! [`authored_changes_holding`]), grouped over the
//! stretch of their part around them rather than the whole part: what [`super::change_groups`]
//! gives of those changes, at the cost of the blocks they reach.
//!
//! Why a stretch is enough: every join of the grouping ties records of one top-level block
//! (a paragraph's tokens and mark, a record and the one it is inside, a table's structure
//! edits) or of two paragraphs that follow one another -- a move's two ends aside, which
//! may stand anywhere in the part. Over a stretch of blocks, the joins of a record in a
//! block strictly between the stretch's first and last blocks holding a paragraph are the
//! joins the whole part has, so a group found with all its records there is the part's
//! group. A group reaching the stretch's edge reads a wider stretch; one holding a move's
//! end reads the whole part.
//!
//! Must not: word a change, or read a part the records are not in.

use super::{Arena, Grouping};
use crate::docx::revisions::{records, Record};
use crate::docx::Docx;
use crate::ids::W;
use ooxml_xml::{Element, Node};
use std::collections::HashSet;
use std::sync::Arc;

/// Blocks read on each side of the records at first.
const MARGIN: usize = 2;

/// The listed changes (`word_changes`' ids: each change's first record's id) holding any of
/// the records at `wanted` (part and place), in story and document order -- the ids of the
/// groups [`super::change_groups`] lists that hold one, read over the stretch of each part
/// those groups reach.
#[must_use]
pub fn changes_holding(docx: &Docx, wanted: &[(String, Vec<u32>)]) -> Vec<String> {
    authored_changes_holding(docx, wanted)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// [`changes_holding`] with each change's author: its first record's that names one (as a
/// [`super::LogicalChange`]'s).
#[must_use]
pub fn authored_changes_holding(docx: &Docx, wanted: &[(String, Vec<u32>)]) -> Vec<(String, Option<String>)> {
    let wanted: HashSet<(&str, &[u32])> = wanted.iter().map(|(p, w)| (p.as_str(), w.as_slice())).collect();
    let all = records(docx);
    let mut out = Vec::new();
    for part in &docx.parts {
        let held: Vec<Record> = all.iter().filter(|r| r.part == part.part).cloned().collect();
        let mine: Vec<usize> = held
            .iter()
            .enumerate()
            .filter(|(_, r)| wanted.contains(&(r.part.as_str(), &*r.path)))
            .map(|(k, _)| k)
            .collect();
        if !mine.is_empty() {
            out.extend(Stretch::new(part.root(), &held).changes(&mine));
        }
    }
    out
}

/// A part's top-level blocks (a body's children, or the root's other children, as
/// [`crate::docx::facts::PartFacts`] takes them) and its records' blocks.
struct Stretch<'r> {
    root: &'r Arc<Element>,
    blocks: Vec<(Box<[u32]>, Arc<Element>)>,
    held: &'r [Record],
    /// Per record, its block.
    block_of: Vec<usize>,
}

impl<'r> Stretch<'r> {
    fn new(root: &'r Arc<Element>, held: &'r [Record]) -> Stretch<'r> {
        let mut blocks = Vec::new();
        for (k, node) in root.children().iter().enumerate() {
            let Node::Element(child) = node else { continue };
            let k = index(k);
            if child.name().is(Some(W), "body") {
                for (j, node) in child.children().iter().enumerate() {
                    if let Node::Element(block) = node {
                        blocks.push((vec![k, index(j)].into_boxed_slice(), Arc::clone(block)));
                    }
                }
            } else {
                blocks.push((vec![k].into_boxed_slice(), Arc::clone(child)));
            }
        }
        // records and blocks are both in document order
        let mut block_of = Vec::with_capacity(held.len());
        let mut b = 0;
        for record in held {
            while blocks.get(b).is_some_and(|(place, _)| !record.path.starts_with(place)) && b + 1 < blocks.len() {
                b += 1;
            }
            block_of.push(b);
        }
        Stretch {
            root,
            blocks,
            held,
            block_of,
        }
    }

    /// The ids and authors of the changes holding the records `mine` (indexes into the
    /// part's records).
    fn changes(&self, mine: &[usize]) -> Vec<(String, Option<String>)> {
        let last = self.blocks.len().saturating_sub(1);
        let blocks: Vec<usize> = mine.iter().filter_map(|k| self.block_of.get(*k).copied()).collect();
        let (low, high) = (
            blocks.iter().copied().min().unwrap_or(0),
            blocks.iter().copied().max().unwrap_or(last),
        );
        let mut margin = MARGIN;
        loop {
            let (a, b) = (low.saturating_sub(margin), (high + margin).min(last));
            match self.read(a, b, mine) {
                Some(found) => return found,
                None if a == 0 && b == last => return Vec::new(),
                None => margin = margin.saturating_mul(4),
            }
        }
    }

    /// The changes holding `mine`, grouped over blocks `a..=b`: `None` when one of them may
    /// reach past the stretch (the whole part is always enough).
    fn read(&self, a: usize, b: usize, mine: &[usize]) -> Option<Vec<(String, Option<String>)>> {
        let whole = a == 0 && b + 1 >= self.blocks.len();
        let first = self.block_of.iter().position(|k| *k >= a).unwrap_or(self.held.len());
        let end = self.block_of.iter().position(|k| *k > b).unwrap_or(self.held.len());
        let held = self.held.get(first..end).unwrap_or_default();
        let mut grouping = Grouping::over(self.arena(a, b), held);
        let (low, high) = (self.holding_paragraph(a..=b), self.holding_paragraph((a..=b).rev()));
        let inside = |k: usize| {
            let block = self.block_of.get(first + k).copied().unwrap_or(a);
            (a == 0 || low.is_some_and(|l| block > l))
                && (b + 1 >= self.blocks.len() || high.is_some_and(|h| block < h))
        };
        let mut out = Vec::new();
        for group in grouping.groups() {
            if !group.members.iter().any(|k| mine.contains(&(first + k))) {
                continue;
            }
            let reaches_out = group
                .members
                .iter()
                .any(|k| !inside(*k) || held.get(*k).is_some_and(|r| super::is_move(&r.element)));
            if reaches_out && !whole {
                return None;
            }
            if let Some(record) = group.members.first().and_then(|k| held.get(*k)) {
                let author = group
                    .members
                    .iter()
                    .filter_map(|k| held.get(*k))
                    .find_map(|r| r.author().map(str::to_owned));
                out.push((record.id.clone(), author));
            }
        }
        Some(out)
    }

    /// The part's tree with only blocks `a..=b` under it (a body's other blocks left out).
    fn arena(&self, a: usize, b: usize) -> Arena {
        let mut arena = Arena::empty(self.root);
        let mut body: Option<(u32, usize)> = None;
        for (place, element) in self.blocks.get(a..=b).unwrap_or_default() {
            let parent = match place.as_ref() {
                [k, _] => match body {
                    Some((at, node)) if at == *k => node,
                    _ => {
                        let found = usize::try_from(*k)
                            .ok()
                            .and_then(|k| self.root.children().iter().nth(k))
                            .and_then(Node::as_element);
                        let node = found.map_or(0, |e| arena.push(e, 0, &[*k]));
                        body = Some((*k, node));
                        node
                    }
                },
                _ => 0,
            };
            arena.add(element, parent, &mut place.to_vec());
        }
        arena
    }

    /// The first block of `range` with a paragraph in it.
    fn holding_paragraph(&self, mut range: impl Iterator<Item = usize>) -> Option<usize> {
        range.find(|k| self.blocks.get(*k).is_some_and(|(_, e)| holds_paragraph(e)))
    }
}

fn holds_paragraph(element: &Element) -> bool {
    element.name().is(Some(W), "p") || element.child_elements().any(|c| holds_paragraph(c))
}

fn index(k: usize) -> u32 {
    u32::try_from(k).unwrap_or(u32::MAX)
}
