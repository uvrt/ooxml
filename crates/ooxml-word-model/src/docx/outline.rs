//! Owns: the outline as docx-agent reads it (`edit/outline.py`, `_OutlineLevels`): which
//! body paragraphs are headings (a built-in `heading N` style's number; else the level the
//! paragraph states, 9 being none; else its style chain's), the body's top-level blocks
//! with their ids, levels and sections, and each heading's range.
//!
//! Must not: change the outline or read text formatting.

use crate::docx::controls::Control;
use crate::docx::{child_val, find_path, style_name, Docx, StoryPart};
use crate::ids::{Entry, W};
use crate::xmlread::py_int;
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// `heading 1` to `heading 9`, in any case: the level.
fn builtin_heading(name: &str) -> Option<i64> {
    let lower = name.to_lowercase();
    let digit = lower.strip_prefix("heading ")?;
    match digit.as_bytes() {
        [d @ b'1'..=b'9'] => Some(i64::from(d - b'0')),
        _ => None,
    }
}

/// `w:outlineLvl`'s value as a level 1-9; `None` for body text (9) or a bad value.
fn outline_value(node: &Element) -> Option<i64> {
    let value = py_int(node.attribute(Some(W), "val").unwrap_or(""))?;
    (0..=8).contains(&value).then_some(value + 1)
}

/// Paragraphs' outline levels, styles read once.
pub struct OutlineLevels<'a> {
    docx: &'a Docx,
    default: Option<String>,
    cache: std::cell::RefCell<HashMap<Option<String>, Option<i64>>>,
}

impl<'a> OutlineLevels<'a> {
    /// The levels of `docx`'s paragraphs.
    #[must_use]
    pub fn new(docx: &'a Docx) -> OutlineLevels<'a> {
        let default = docx
            .default_style("paragraph")
            .map(|s| crate::docx::style_id(s).to_owned());
        OutlineLevels {
            docx,
            default,
            cache: std::cell::RefCell::default(),
        }
    }

    /// The paragraph style applied: the one named where the document has it, else the default.
    #[must_use]
    pub fn style_id(&self, paragraph: &Element) -> Option<String> {
        let named = find_path(paragraph, &["pPr", "pStyle"]).and_then(|s| s.attribute(Some(W), "val"));
        match named {
            Some(named) if self.docx.style(named).is_some() => Some(named.to_owned()),
            _ => self.default.clone(),
        }
    }

    /// The level a paragraph style gives through its chain.
    #[must_use]
    pub fn style(&self, style_id: Option<&str>) -> Option<i64> {
        let key = style_id.map(str::to_owned);
        if let Some(found) = self.cache.borrow().get(&key) {
            return *found;
        }
        let level = style_outline_level(self.docx, style_id);
        self.cache.borrow_mut().insert(key, level);
        level
    }

    /// The paragraph's level: a built-in heading style's number; else the level it states;
    /// else its style's.
    #[must_use]
    pub fn effective(&self, paragraph: &Element) -> Option<i64> {
        let style_id = self.style_id(paragraph);
        let node = style_id.as_deref().and_then(|s| self.docx.style(s));
        if let Some(level) = node.and_then(|n| builtin_heading(style_name(n).unwrap_or(""))) {
            return Some(level);
        }
        if let Some(stated) = find_path(paragraph, &["pPr", "outlineLvl"]) {
            return outline_value(stated);
        }
        self.style(style_id.as_deref())
    }
}

/// The outline level a paragraph style gives: a built-in heading's number, else the first
/// `w:outlineLvl` its `basedOn` chain declares.
#[must_use]
pub fn style_outline_level(docx: &Docx, style_id: Option<&str>) -> Option<i64> {
    let mut current = style_id.map(str::to_owned);
    let mut seen = Vec::new();
    while let Some(id) = current.take() {
        if id.is_empty() || seen.contains(&id) {
            break;
        }
        let Some(node) = docx.style(&id) else { break };
        seen.push(id);
        if let Some(level) = builtin_heading(style_name(node).unwrap_or("")) {
            return Some(level);
        }
        if let Some(stated) = find_path(node, &["pPr", "outlineLvl"]) {
            return outline_value(stated);
        }
        current = child_val(node, "basedOn").map(str::to_owned);
    }
    None
}

/// One top-level block of the body.
#[derive(Debug, Clone)]
pub struct Block {
    /// Its id (`None` for custom XML).
    pub id: Option<String>,
    /// Its outline level, for a paragraph that is a heading.
    pub level: Option<i64>,
    /// The section it is in, from 1.
    pub section: usize,
    /// The element.
    pub element: Arc<Element>,
}

/// A heading and its range.
#[derive(Debug, Clone)]
pub struct Heading {
    /// Its id.
    pub id: String,
    /// Its level, 1-9.
    pub level: i64,
    /// How many blocks it owns, itself included.
    pub blocks: usize,
    /// The section it is in.
    pub section: usize,
    /// The heading paragraph.
    pub element: Arc<Element>,
    /// Its place among the body's blocks, and one past its last.
    pub start: usize,
    /// One past its last block.
    pub end: usize,
    /// Its last block's id.
    pub last: String,
}

/// Every top-level block of the body in order.
#[must_use]
pub fn body_blocks(docx: &Docx, controls: &[Control]) -> Vec<Block> {
    let Some(body_part) = docx.body() else {
        return Vec::new();
    };
    let levels = OutlineLevels::new(docx);
    let mut out = Vec::new();
    let Some((body_index, body)) = child_of(body_part.root(), "body") else {
        return out;
    };
    let mut section = 1;
    for (k, node) in body.children().iter().enumerate() {
        let Node::Element(node) = node else { continue };
        let path = [body_index, u32::try_from(k).unwrap_or(u32::MAX)];
        let name = node.name();
        let (mut id, mut level, mut breaks) = (None, None, 0);
        if name.is(Some(W), "p") || name.is(Some(W), "tbl") {
            id = entry_id(body_part, &path);
            if name.is(Some(W), "p") {
                level = levels.effective(node);
                breaks = usize::from(find_path(node, &["pPr", "sectPr"]).is_some());
            }
        } else if name.is(Some(W), "sdt") || name.is(Some(W), "customXml") {
            if name.is(Some(W), "sdt") {
                id = controls
                    .iter()
                    .find(|c| c.part == body_part.part && *c.path == path)
                    .map(|c| c.id.clone());
            }
            breaks = paragraph_breaks(node);
        } else {
            continue;
        }
        out.push(Block {
            id,
            level,
            section,
            element: Arc::clone(node),
        });
        section += breaks;
    }
    out
}

/// The body's headings with their ranges, in order.
#[must_use]
pub fn headings(blocks: &[Block]) -> Vec<Heading> {
    let mut out = Vec::new();
    for (k, block) in blocks.iter().enumerate() {
        let (Some(level), Some(id)) = (block.level, block.id.as_ref()) else {
            continue;
        };
        let end = (k + 1..blocks.len())
            .find(|j| blocks.get(*j).and_then(|b| b.level).is_some_and(|l| l <= level))
            .unwrap_or(blocks.len());
        let last = (k + 1..end)
            .rev()
            .find(|j| blocks.get(*j).is_some_and(|b| b.id.is_some()))
            .unwrap_or(k);
        out.push(Heading {
            id: id.clone(),
            level,
            blocks: end - k,
            section: block.section,
            element: Arc::clone(&block.element),
            start: k,
            end,
            last: blocks
                .get(last)
                .and_then(|b| b.id.clone())
                .unwrap_or_else(|| id.clone()),
        });
    }
    out
}

/// The first child of `root` named `w:{local}`, with its index.
#[must_use]
pub fn child_of<'a>(root: &'a Arc<Element>, local: &str) -> Option<(u32, &'a Arc<Element>)> {
    root.children().iter().enumerate().find_map(|(k, n)| match n {
        Node::Element(e) if e.name().is(Some(W), local) => Some((u32::try_from(k).ok()?, e)),
        _ => None,
    })
}

fn entry_id(part: &StoryPart, path: &[u32]) -> Option<String> {
    let entry: Entry = part.ids.at(path)?;
    part.ids.id_of(entry).map(str::to_owned)
}

/// Section breaks in a block-level control or custom XML: `w:p/w:pPr/w:sectPr` below it.
fn paragraph_breaks(node: &Element) -> usize {
    let mut count = 0;
    let mut stack = vec![node];
    while let Some(e) = stack.pop() {
        for child in e.child_elements() {
            if child.name().is(Some(W), "p") {
                for ppr in child.child_elements().filter(|p| p.name().is(Some(W), "pPr")) {
                    count += ppr.child_elements().filter(|s| s.name().is(Some(W), "sectPr")).count();
                }
            }
            stack.push(child);
        }
    }
    count
}
