//! Owns: what a change touched, for the application (docx-agent's `tools/touched.py`,
//! `Result.touched`): every part whose content the change replaced, and for the body the
//! heading sections that hold the blocks it changed -- derived from the version's change set
//! ([`VersionDelta`]: the parts that differ, the body's blocks by key), not by reading the
//! document again. A block whose only change is the ids stamping writes counts as
//! unchanged, as docx-agent's digests leave those ids out.
//!
//! Must not: read a block that did not change, or lay anything out.

use crate::doc::WordDoc;
use crate::names::W;
use crate::Result;
use ooxml_word_model::docx::{live_paths, Docx};
use ooxml_word_model::ids::{BlockDelta, BlockKey, PartIds, StoryDelta, VersionDelta};
use ooxml_word_model::markdown::stylemap::StyleMap;
use ooxml_xml::Element;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// One entry of `touched`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Touched {
    /// The part.
    pub part: ooxml_opc::PartName,
    /// For a body section: its heading's id (`None` before the first heading), its level
    /// (0 there), and its first and last block.
    pub section: Option<Section>,
}

/// A heading section of the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The heading's id, `None` for the blocks before the first heading.
    pub heading: Option<String>,
    /// Its level (1-9), 0 for the blocks before the first heading.
    pub level: i64,
    /// The section's first and last block.
    pub blocks: [String; 2],
}

const BLOCKS: &[&str] = &["p", "tbl", "sdt", "customXml"];

fn is_block(e: &Element) -> bool {
    e.name().namespace() == Some(W) && BLOCKS.contains(&e.name().local())
}

/// A version's body, read where asked: its blocks by their place among the body's
/// children (every node counted).
struct Body<'a> {
    /// The body's place among the root's children.
    at: u32,
    children: &'a ooxml_xml::Children,
}

impl<'a> Body<'a> {
    fn of(root: &'a Element) -> Option<Body<'a>> {
        let (at, body) = root
            .children()
            .iter()
            .enumerate()
            .find_map(|(k, n)| n.as_element().filter(|e| e.name().is(Some(W), "body")).map(|e| (k, e)))?;
        Some(Body {
            at: u32::try_from(at).ok()?,
            children: body.children(),
        })
    }

    /// The block at place `k`.
    fn block(&self, k: usize) -> Option<&'a Arc<Element>> {
        self.children.get(k)?.as_element().filter(|e| is_block(e))
    }

    /// The place of the last block before place `k`.
    fn block_before(&self, k: usize) -> Option<usize> {
        (0..k).rev().find(|j| self.block(*j).is_some())
    }

    /// The place of the first block after place `k`.
    fn block_after(&self, k: usize) -> Option<usize> {
        (k + 1..self.children.len()).find(|j| self.block(*j).is_some())
    }

    /// The place of the first block.
    fn first_block(&self) -> Option<usize> {
        self.children
            .iter()
            .position(|n| n.as_element().is_some_and(|e| is_block(e)))
    }

    /// How many blocks come before place `k` (a block's number among the body's).
    fn number(&self, k: usize) -> usize {
        self.children
            .iter()
            .take(k)
            .filter(|n| n.as_element().is_some_and(|e| is_block(e)))
            .count()
    }
}

/// The heading levels of the body's paragraphs, as docx-agent's `_Levels` reads them:
/// a paragraph's own outline level, else its style's (by name, then its chain's outline
/// level), the default paragraph style's for a paragraph that names none.
struct Levels<'d> {
    docx: &'d Docx,
    map: StyleMap,
    memo: HashMap<Option<String>, Option<i64>>,
}

impl Levels<'_> {
    fn of(&mut self, block: &Element) -> Option<i64> {
        if !block.name().is(Some(W), "p") {
            return None;
        }
        let properties = block.find(Some(W), "pPr");
        if let Some(direct) = properties.and_then(|p| p.find(Some(W), "outlineLvl")) {
            let level = direct
                .attribute(Some(W), "val")
                .and_then(|v| v.trim().parse::<i64>().ok());
            return level.filter(|l| (0..=8).contains(l)).map(|l| l + 1);
        }
        let style = properties
            .and_then(|p| p.find(Some(W), "pStyle"))
            .and_then(|s| s.attribute(Some(W), "val"))
            .map(str::to_owned);
        if let Some(found) = self.memo.get(&style) {
            return *found;
        }
        let level = self.style_level(style.as_deref());
        self.memo.insert(style, level);
        level
    }

    fn style_level(&self, style_id: Option<&str>) -> Option<i64> {
        let default = self
            .docx
            .styles()
            .iter()
            .find(|s| {
                s.attribute(Some(W), "type") == Some("paragraph")
                    && matches!(s.attribute(Some(W), "default"), Some("1" | "true" | "on"))
            })
            .and_then(|s| s.attribute(Some(W), "styleId"))
            .map(str::to_owned);
        let mut id = style_id.map(str::to_owned);
        let mut node = id.as_deref().and_then(|i| self.docx.style(i));
        if node.is_none() {
            id.clone_from(&default);
            node = id.as_deref().and_then(|i| self.docx.style(i));
        }
        let name = node
            .and_then(|n| n.find(Some(W), "name"))
            .and_then(|n| n.attribute(Some(W), "val"))
            .map(str::to_owned);
        let mut outline = None;
        let mut current = id;
        let mut seen = BTreeSet::new();
        while let Some(at) = current.clone().filter(|c| !seen.contains(c)) {
            let Some(style) = self.docx.style(&at) else { break };
            seen.insert(at);
            if let Some(level) = style.find(Some(W), "pPr").and_then(|p| p.find(Some(W), "outlineLvl")) {
                outline = level
                    .attribute(Some(W), "val")
                    .and_then(|v| v.trim().parse::<i64>().ok());
                break;
            }
            current = style
                .find(Some(W), "basedOn")
                .and_then(|b| b.attribute(Some(W), "val"))
                .map(str::to_owned);
        }
        self.map.heading_level(name.as_deref(), outline)
    }
}

/// The ids of the body's blocks as docx-agent names them -- a paragraph's or table's id,
/// a content control's `cc:` id, else `b@body/<n>` -- read for the blocks asked for only.
struct BlockIds<'a> {
    docx: &'a Docx,
    body: &'a Body<'a>,
    /// Content controls' ids by element, read on the first control asked for.
    sdts: Option<HashMap<usize, String>>,
}

impl<'a> BlockIds<'a> {
    fn new(docx: &'a Docx, body: &'a Body<'a>) -> Self {
        BlockIds { docx, body, sdts: None }
    }

    /// The id of the block at place `k`.
    fn of(&mut self, k: usize) -> String {
        let (Some(story), Some(block), Ok(place)) = (self.docx.body(), self.body.block(k), u32::try_from(k)) else {
            return format!("b@body/{}", self.body.number(k));
        };
        if let Some(id) = story.ids.at(&[self.body.at, place]).and_then(|e| story.ids.id_of(e)) {
            return id;
        }
        let sdts = self.sdts.get_or_insert_with(|| control_ids(story.root()));
        sdts.get(&(Arc::as_ptr(block) as usize))
            .cloned()
            .unwrap_or_else(|| format!("b@body/{}", self.body.number(k)))
    }
}

/// `cc:<id>` (`#k` for a repeat) or `cc@body/<n>` of every live content control.
fn control_ids(root: &Element) -> HashMap<usize, String> {
    let mut out = HashMap::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (number, sdt) in live_paths(root, &[], &|e| e.name().is(Some(W), "sdt"))
        .iter()
        .enumerate()
    {
        let raw = sdt
            .element
            .find(Some(W), "sdtPr")
            .and_then(|p| p.find(Some(W), "id"))
            .and_then(|i| i.attribute(Some(W), "val"))
            .map(str::to_owned);
        let id = match raw {
            None => format!("cc@body/{number}"),
            Some(raw) => {
                let count = seen.entry(raw.clone()).or_insert(0);
                let id = if *count > 0 {
                    format!("cc:{raw}#{count}")
                } else {
                    format!("cc:{raw}")
                };
                *count += 1;
                id
            }
        };
        out.insert(Arc::as_ptr(&sdt.element) as usize, id);
    }
    out
}

/// What the change from `before` to `after` touched, one entry per part (sorted by name),
/// the body's per heading section.
///
/// # Errors
/// When `after` cannot be read as a Word document.
pub fn touched(before: &WordDoc, after: &WordDoc) -> Result<Vec<Touched>> {
    touched_of(before, after, &after.delta_from(before))
}

/// [`touched`], from `delta`, what `after` changed against `before`.
///
/// # Errors
/// When `before` or `after` cannot be read as a Word document.
pub fn touched_of(before: &WordDoc, after: &WordDoc, delta: &VersionDelta) -> Result<Vec<Touched>> {
    let docx = after.docx()?;
    let mut out = Vec::new();
    for part in delta.changed_parts() {
        if part == docx.main {
            if let Some(sections) = body_sections(before, &docx, delta)? {
                out.extend(sections.into_iter().map(|section| Touched {
                    part: part.to_owned().into(),
                    section,
                }));
                continue;
            }
        }
        out.push(Touched {
            part: part.to_owned().into(),
            section: None,
        });
    }
    Ok(out)
}

/// The body's changed heading sections, and a plain entry when something outside the
/// blocks changed too (or no section did); `None` when either version has no body.
fn body_sections(before: &WordDoc, docx: &Docx, delta: &VersionDelta) -> Result<Option<Vec<Option<Section>>>> {
    let old_docx = before.docx()?;
    let (Some(old_story), Some(new_story)) = (old_docx.body(), docx.body()) else {
        return Ok(None);
    };
    let (Some(_), Some(new)) = (Body::of(old_story.root()), Body::of(new_story.root())) else {
        return Ok(None);
    };
    let blocks = delta
        .stories
        .iter()
        .find(|s| s.part == docx.main)
        .and_then(|s| match &s.delta {
            StoryDelta::Blocks(blocks) => Some(blocks),
            StoryDelta::Whole => None,
        });
    let Some(blocks) = blocks else {
        return Ok(Some(vec![None]));
    };
    let changed = changed_blocks(&old_story.ids, &new_story.ids, &new, blocks);
    let mut out = if changed.is_empty() {
        Vec::new()
    } else {
        sections(docx, &new, &changed)
    };
    if blocks.outside || out.is_empty() {
        out.push(None);
    }
    Ok(Some(out))
}

/// The places of the new body's blocks (its container children) the change touched, from
/// the body's change set: those holding a block that changed, came, moved or whose wrapping
/// changed; and for a block gone (or moved away), the block before where it was -- unless a
/// block touched follows there (it was replaced), or it was inside a block both versions
/// hold (that block changed). A block changed only by stamping counts as unchanged.
fn changed_blocks(old_ids: &PartIds, new_ids: &PartIds, new: &Body<'_>, delta: &BlockDelta) -> BTreeSet<usize> {
    let top = |ids: &PartIds, key: BlockKey| -> Option<usize> {
        let place = ids.place_of(key)?;
        let c = ids.container().len();
        (place.len() > c && place.starts_with(ids.container()))
            .then(|| place.get(c).map(|k| *k as usize))
            .flatten()
    };
    let in_new = |key: BlockKey| top(new_ids, key).filter(|k| new.block(*k).is_some());
    let mut changed: BTreeSet<usize> = delta
        .changed
        .iter()
        .copied()
        .chain(delta.came.iter().map(|(k, _)| *k))
        .chain(delta.wrapped.iter().copied())
        .chain(delta.moved.iter().map(|m| m.key))
        .filter_map(in_new)
        .collect();
    let old_key = |key: BlockKey| delta.renamed.iter().find(|(_, n)| *n == key).map_or(key, |(o, _)| *o);
    let removals = delta
        .gone
        .iter()
        .copied()
        .chain(delta.moved.iter().map(|m| (old_key(m.key), m.was_after)));
    let mut at_removals = BTreeSet::new();
    for (gone, was_after) in removals {
        let Some(before) = was_after else {
            // removed at the body's start: counts at its first block, unless that changed
            if let Some(first) = new.first_block().filter(|k| !changed.contains(k)) {
                at_removals.insert(first);
            }
            continue;
        };
        let Some(t) = in_new(before) else { continue };
        let inside = top(old_ids, gone).is_some_and(|g| top(old_ids, old_key(before)) == Some(g));
        let replaced = new.block_after(t).is_some_and(|next| changed.contains(&next));
        if inside || !replaced {
            at_removals.insert(t);
        }
    }
    changed.append(&mut at_removals);
    changed
}

/// The heading sections holding the `changed` blocks (by place): from the heading at or
/// before each to the next heading of its level or above. Levels are read walking out
/// from the changed blocks only.
fn sections(docx: &Docx, new: &Body<'_>, changed: &BTreeSet<usize>) -> Vec<Option<Section>> {
    let mut levels = Levels {
        docx,
        map: StyleMap::default(),
        memo: HashMap::new(),
    };
    // Each block's level read once: the walks below pass a block once per changed block
    // and per section start that reaches it.
    let mut known: HashMap<usize, Option<i64>> = HashMap::new();
    let mut level = |k: usize| {
        *known
            .entry(k)
            .or_insert_with(|| new.block(k).and_then(|b| levels.of(b)))
    };
    let mut starts = BTreeSet::new();
    // The changed blocks in order: a walk back that reaches the changed block before has
    // that block's heading (no heading stands between them), so the walks back pass each
    // block once together.
    let mut previous: Option<(usize, Option<usize>)> = None;
    for &j in changed {
        let mut k = Some(j);
        let mut reached = None;
        while let Some(at) = k.filter(|at| level(*at).is_none()) {
            if let Some((_, start)) = previous.filter(|(place, _)| *place == at && at != j) {
                reached = Some(start);
                break;
            }
            k = new.block_before(at);
        }
        let start = match reached {
            Some(start) => start,
            None => k.or_else(|| new.first_block()),
        };
        if let Some(start) = start {
            starts.insert(start);
        }
        previous = Some((j, start));
    }
    let mut ids = BlockIds::new(docx, new);
    let mut out = Vec::new();
    for start in starts {
        let own = level(start);
        let mut end = None;
        let mut at = new.block_after(start);
        while let Some(k) = at {
            if level(k).is_some_and(|l| own.is_none_or(|o| l <= o)) {
                break;
            }
            end = Some(k);
            at = new.block_after(k);
        }
        let last = end.unwrap_or(start);
        out.push(Some(Section {
            heading: own.map(|_| ids.of(start)),
            level: own.unwrap_or(0),
            blocks: [ids.of(start), ids.of(last)],
        }));
    }
    out
}
