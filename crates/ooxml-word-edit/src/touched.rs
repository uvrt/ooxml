//! Owns: what a change touched, for the application (docx-agent's `tools/touched.py`,
//! `Result.touched`): every part whose content the change replaced, and for the body the
//! heading sections that hold the blocks it changed -- found from the change itself (the
//! parts it wrote, the body's blocks it replaced, compared by identity), not by reading the
//! document again. A block whose only change is the ids stamping writes counts as
//! unchanged, as docx-agent's digests leave those ids out.
//!
//! Must not: read a block that did not change, or lay anything out.

use crate::doc::WordDoc;
use crate::names::{W, W14, WP14};
use crate::text::diff::{opcodes, Tag};
use crate::Result;
use ooxml_word_model::docx::{live_paths, Docx};
use ooxml_word_model::markdown::stylemap::StyleMap;
use ooxml_xml::{Element, Node};
use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// One entry of `touched`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Touched {
    /// The part.
    pub part: String,
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

/// A block's content without the ids stamping writes, as a hash: equal for blocks docx-agent's digests find equal.
pub(crate) fn stampless(element: &Element) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hash_element(element, &mut hasher);
    hasher.finish()
}

fn hash_element(element: &Element, h: &mut impl Hasher) {
    element.name().namespace().hash(h);
    element.name().local().hash(h);
    // declarations are part of the bytes docx-agent digests
    for d in element.namespaces() {
        d.prefix.hash(h);
        d.uri.hash(h);
    }
    for attribute in element.attributes() {
        let n = &attribute.name;
        let stamp = (n.namespace() == Some(W14) && (n.local() == "paraId" || n.local() == "textId"))
            || (n.namespace() == Some(WP14) && (n.local() == "anchorId" || n.local() == "editId"));
        if !stamp {
            n.namespace().hash(h);
            n.local().hash(h);
            attribute.value.hash(h);
        }
    }
    for node in element.children().iter() {
        match node {
            Node::Element(child) => {
                1u8.hash(h);
                hash_element(child, h);
            }
            Node::Text(t) => {
                2u8.hash(h);
                t.hash(h);
            }
            Node::Comment(c) => {
                3u8.hash(h);
                c.hash(h);
            }
            Node::Pi(p) => {
                4u8.hash(h);
                p.target.hash(h);
                p.data.hash(h);
            }
            Node::EntityRef(r) => {
                5u8.hash(h);
                r.hash(h);
            }
        }
    }
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

    /// The blocks in places `from..to`, with their places.
    fn blocks(&self, from: usize, to: usize) -> Vec<(usize, &'a Arc<Element>)> {
        self.children
            .iter()
            .enumerate()
            .skip(from)
            .take(to - from)
            .filter_map(|(k, n)| n.as_element().filter(|e| is_block(e)).map(|e| (k, e)))
            .collect()
    }

    /// The nodes that are not blocks in places `from..to`.
    fn gaps(&self, from: usize, to: usize) -> Vec<&'a Node> {
        self.children
            .iter()
            .skip(from)
            .take(to - from)
            .filter(|n| !n.as_element().is_some_and(|e| is_block(e)))
            .collect()
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
    let docx = after.docx()?;
    let mut names: BTreeSet<String> = before.package().part_names().into_iter().collect();
    names.extend(after.package().part_names());
    let mut out = Vec::new();
    for part in names {
        let same = match (before.package().part(&part), after.package().part(&part)) {
            (Some(a), Some(b)) => a.same(b),
            (None, None) => true,
            _ => false,
        };
        if same {
            continue;
        }
        if part == docx.main {
            if let Some(sections) = body_sections(before, after, &docx)? {
                out.extend(sections.into_iter().map(|section| Touched {
                    part: part.clone(),
                    section,
                }));
                continue;
            }
        }
        out.push(Touched { part, section: None });
    }
    Ok(out)
}

/// The body's changed heading sections, and a plain entry when something outside the
/// blocks changed too (or no section did); `None` when either version has no body.
fn body_sections(before: &WordDoc, after: &WordDoc, docx: &Docx) -> Result<Option<Vec<Option<Section>>>> {
    let (Some(old_tree), Some(new_tree)) = (before.tree(&docx.main)?, after.tree(&docx.main)?) else {
        return Ok(None);
    };
    let (Some(old), Some(new)) = (Body::of(old_tree.root()), Body::of(new_tree.root())) else {
        return Ok(None);
    };
    let Some((start, old_end, new_end)) = old.children.changed_span(new.children) else {
        return Ok(Some(vec![None]));
    };
    let changed = changed_blocks(
        after,
        &new,
        (&old.blocks(start, old_end), &new.blocks(start, new_end)),
        start,
    );
    let mut out = if changed.is_empty() {
        Vec::new()
    } else {
        sections(docx, &new, &changed)
    };
    let (old_gaps, new_gaps) = (old.gaps(start, old_end), new.gaps(start, new_end));
    let same_rest = old_gaps.len() == new_gaps.len() && old_gaps.iter().zip(&new_gaps).all(|(x, y)| x == y);
    if !same_rest || out.is_empty() {
        out.push(None);
    }
    Ok(Some(out))
}

/// The blocks of a stretch of the body, with their places.
type Stretch<'a> = [(usize, &'a Arc<Element>)];

/// The places of the new body's blocks the change replaced, from the stretch of children
/// that differs (the blocks before and after it are the same allocations). Within it the
/// blocks both versions hold (the same allocation, in the same order) are untouched; only
/// the pieces between them are compared, by content without stamps; a removal counts at the
/// block before it. A change that rewrites scattered blocks so compares each one with what
/// it replaced, never the whole body.
fn changed_blocks(
    after: &WordDoc,
    new: &Body<'_>,
    (old, now): (&Stretch<'_>, &Stretch<'_>),
    start: usize,
) -> BTreeSet<usize> {
    let facts = &after.shared().facts;
    let digest = |b: &Arc<Element>| facts.element(b).stampless;
    let mut changed = BTreeSet::new();
    // a removal at the stretch's first block counts at the block before the stretch
    let before_stretch = || new.block_before(start).or_else(|| new.first_block());
    for piece in pieces(old, now) {
        let (a, b): (Vec<u64>, Vec<u64>) = (
            old.get(piece.old.clone()).unwrap_or_default().iter().map(|(_, e)| digest(e)).collect(),
            now.get(piece.new.clone()).unwrap_or_default().iter().map(|(_, e)| digest(e)).collect(),
        );
        let place = |j: usize| now.get(piece.new.start + j).map(|(k, _)| *k);
        for op in opcodes(&a, &b) {
            if op.tag == Tag::Equal {
                continue;
            }
            if op.j2 > op.j1 {
                changed.extend((op.j1..op.j2).filter_map(place));
            } else {
                // a removal counts at the block before it (the body's first block when none)
                let at = match (piece.new.start + op.j1).checked_sub(1) {
                    Some(j) => now.get(j).map(|(k, _)| *k),
                    None => before_stretch(),
                };
                if let Some(k) = at {
                    changed.insert(k);
                }
            }
        }
    }
    changed
}

/// A piece of the stretch between two blocks both versions hold: its old blocks' and new
/// blocks' indexes.
struct Piece {
    old: std::ops::Range<usize>,
    new: std::ops::Range<usize>,
}

/// The pieces of `old` and `now` between the blocks both hold (the same allocation), taken
/// in order: each new block found among the old ones after the last such block is one.
/// Pieces where nothing differs are left out.
fn pieces(old: &Stretch<'_>, now: &Stretch<'_>) -> Vec<Piece> {
    let at: HashMap<*const Element, usize> = old.iter().enumerate().map(|(i, (_, e))| (Arc::as_ptr(e), i)).collect();
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    for (k, (_, e)) in now.iter().enumerate() {
        let Some(&found) = at.get(&Arc::as_ptr(e)) else { continue };
        if found < i {
            continue; // held by both, but moved: compared within its piece
        }
        if found > i || k > j {
            out.push(Piece { old: i..found, new: j..k });
        }
        (i, j) = (found + 1, k + 1);
    }
    if i < old.len() || j < now.len() {
        out.push(Piece { old: i..old.len(), new: j..now.len() });
    }
    out
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
    let mut level = |k: usize| new.block(k).and_then(|b| levels.of(b));
    let mut starts = BTreeSet::new();
    for &j in changed {
        let mut k = Some(j);
        while let Some(at) = k.filter(|at| level(*at).is_none()) {
            k = new.block_before(at);
        }
        if let Some(k) = k.or_else(|| new.first_block()) {
            starts.insert(k);
        }
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
