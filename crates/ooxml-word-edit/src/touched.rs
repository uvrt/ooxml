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

/// The body's children: blocks with their places, and what is between them.
struct Body {
    /// The body's place among the root's children.
    at: u32,
    blocks: Vec<Arc<Element>>,
    places: Vec<u32>,
    gaps: Vec<Node>,
}

fn body_of(root: &Element) -> Option<Body> {
    let (at, body) = root
        .children()
        .iter()
        .enumerate()
        .find_map(|(k, n)| n.as_element().filter(|e| e.name().is(Some(W), "body")).map(|e| (k, e)))?;
    let mut out = Body {
        at: u32::try_from(at).ok()?,
        blocks: Vec::new(),
        places: Vec::new(),
        gaps: Vec::new(),
    };
    for (k, node) in body.children().iter().enumerate() {
        match node {
            Node::Element(e) if is_block(e) => {
                out.blocks.push(Arc::clone(e));
                out.places.push(u32::try_from(k).unwrap_or(u32::MAX));
            }
            other => out.gaps.push(other.clone()),
        }
    }
    Some(out)
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
    body: &'a Body,
    /// Content controls' ids by element, read on the first control asked for.
    sdts: Option<HashMap<usize, String>>,
}

impl<'a> BlockIds<'a> {
    fn new(docx: &'a Docx, body: &'a Body) -> Self {
        BlockIds { docx, body, sdts: None }
    }

    fn of(&mut self, k: usize) -> String {
        let (Some(story), Some(block), Some(place)) =
            (self.docx.body(), self.body.blocks.get(k), self.body.places.get(k))
        else {
            return format!("b@body/{k}");
        };
        if let Some(id) = story.ids.at(&[self.body.at, *place]).and_then(|e| story.ids.id_of(e)) {
            return id.to_owned();
        }
        let sdts = self.sdts.get_or_insert_with(|| control_ids(story.root()));
        sdts.get(&(Arc::as_ptr(block) as usize))
            .cloned()
            .unwrap_or_else(|| format!("b@body/{k}"))
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
    let (Some(old), Some(new)) = (body_of(old_tree.root()), body_of(new_tree.root())) else {
        return Ok(None);
    };
    let changed = changed_blocks(after, &old, &new);
    let mut out = if changed.is_empty() {
        Vec::new()
    } else {
        sections(docx, &new, &changed)
    };
    let same_rest = old.gaps.len() == new.gaps.len() && old.gaps.iter().zip(&new.gaps).all(|(x, y)| x == y);
    if !same_rest || out.is_empty() {
        out.push(None);
    }
    Ok(Some(out))
}

/// The places of the new body's blocks the change replaced: those that are not the old
/// version's (by pointer, before and after the stretch that changed), compared within
/// that stretch by content without stamps; a removal counts at the block before it.
fn changed_blocks(after: &WordDoc, old: &Body, new: &Body) -> BTreeSet<usize> {
    let facts = &after.shared().facts;
    let digest = |b: &Arc<Element>| facts.element(b).stampless;
    let prefix = old
        .blocks
        .iter()
        .zip(&new.blocks)
        .take_while(|(a, b)| Arc::ptr_eq(a, b))
        .count();
    let rest = |blocks: &[Arc<Element>]| blocks.get(prefix..).unwrap_or_default().to_vec();
    let (old_rest, new_rest) = (rest(&old.blocks), rest(&new.blocks));
    let suffix = old_rest
        .iter()
        .rev()
        .zip(new_rest.iter().rev())
        .take_while(|(a, b)| Arc::ptr_eq(a, b))
        .count();
    let middle = |blocks: &[Arc<Element>]| -> Vec<u64> {
        blocks
            .get(..blocks.len() - suffix)
            .unwrap_or_default()
            .iter()
            .map(digest)
            .collect()
    };
    let (a, b) = (middle(&old_rest), middle(&new_rest));
    let mut changed = BTreeSet::new();
    for op in opcodes(&a, &b) {
        if op.tag == Tag::Equal {
            continue;
        }
        if op.j2 > op.j1 {
            changed.extend(prefix + op.j1..prefix + op.j2);
        } else if !new.blocks.is_empty() {
            changed.insert((prefix + op.j1).saturating_sub(1).min(new.blocks.len() - 1));
        }
    }
    changed
}

/// The heading sections holding the `changed` blocks: from the heading at or before each
/// to the next heading of its level or above. Levels are read walking out from the
/// changed blocks only.
fn sections(docx: &Docx, new: &Body, changed: &BTreeSet<usize>) -> Vec<Option<Section>> {
    let mut levels = Levels {
        docx,
        map: StyleMap::default(),
        memo: HashMap::new(),
    };
    let mut level = |k: usize| new.blocks.get(k).and_then(|b| levels.of(b));
    let mut starts = BTreeSet::new();
    for &j in changed {
        let mut k = Some(j);
        while let Some(at) = k.filter(|at| level(*at).is_none()) {
            k = at.checked_sub(1);
        }
        starts.insert(k.unwrap_or(0));
    }
    let mut ids = BlockIds::new(docx, new);
    let mut out = Vec::new();
    for start in starts {
        let own = level(start);
        let end = (start + 1..new.blocks.len())
            .find(|k| level(*k).is_some_and(|l| own.is_none_or(|o| l <= o)))
            .unwrap_or(new.blocks.len());
        out.push(Some(Section {
            heading: own.map(|_| ids.of(start)),
            level: own.unwrap_or(0),
            blocks: [ids.of(start), ids.of(end.saturating_sub(1))],
        }));
    }
    out
}
