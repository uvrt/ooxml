//! Owns: the body's list labels kept up to date across versions (the labels' tree changed
//! by the blocks that changed between the unchanged ones at either end), and the labels of
//! the other stories counted from scratch: the headers' and footers', and the text boxes'
//! (one story of their own).
//!
//! Must not: count labels itself (`crate::labels` does).

use super::Model;
use crate::document::{item_of, Context, WordDocument};
use crate::labels::{item_lists, top_level, BlockItems, Counters};
use crate::model::{Block, Paragraph};
use crate::value::{text, Text};
use ooxml_base::{Digest, DigestKey, Digester};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// The key of a block's list items: what decides each paragraph's resolved `numPr`.
fn label_key(key: DigestKey, block: &Block) -> Digest {
    fn feed(d: &mut Digester, block: &Block) {
        match block {
            Block::Paragraph(p) => {
                d.update(b"p");
                let props = p.properties.as_deref();
                d.field(props.and_then(|x| x.style_id.as_deref()).unwrap_or("\u{0}").as_bytes());
                d.field(p.table_style_id.as_deref().unwrap_or("\u{0}").as_bytes());
                for c in p.table_conditions.iter() {
                    d.field(c.as_bytes());
                }
                d.update(b"|");
                for name in ["numPr.numId", "numPr.ilvl"] {
                    match props
                        .and_then(|x| x.declared.get(name))
                        .and_then(crate::value::Value::int)
                    {
                        Some(n) => d.field(&n.to_le_bytes()),
                        None => d.field(b""),
                    }
                }
            }
            Block::Table(t) => {
                d.update(b"t");
                for row in t.rows.iter() {
                    for cell in row.iter() {
                        d.update(b"c");
                        cell.iter().for_each(|b| feed(d, b));
                    }
                }
                d.update(b"/");
            }
        }
    }
    let mut d = Digester::new(key);
    feed(&mut d, block);
    d.finish()
}

/// The label keys of the last version's blocks by the block (its pointer, the block held):
/// a block the next version shares is not digested again.
#[derive(Default)]
pub(super) struct LabelKeys {
    last: HashMap<usize, (Block, Digest)>,
    now: HashMap<usize, (Block, Digest)>,
}

fn block_pointer(block: &Block) -> usize {
    match block {
        Block::Paragraph(p) => Arc::as_ptr(p) as usize,
        Block::Table(t) => Arc::as_ptr(t) as usize,
    }
}

fn same_pointer(a: &Block, b: &Block) -> bool {
    match (a, b) {
        (Block::Paragraph(x), Block::Paragraph(y)) => Arc::ptr_eq(x, y),
        (Block::Table(x), Block::Table(y)) => Arc::ptr_eq(x, y),
        _ => false,
    }
}

impl LabelKeys {
    /// `block`'s label key, kept for the next version.
    fn of(&mut self, key: DigestKey, block: &Block) -> Digest {
        let at = block_pointer(block);
        let d = match self.last.get(&at) {
            Some((kept, d)) if same_pointer(kept, block) => *d,
            _ => label_key(key, block),
        };
        self.now.insert(at, (block.clone(), d));
        d
    }

    /// The version's blocks keyed: the next version is keyed from them.
    fn finish(&mut self) {
        self.last = std::mem::take(&mut self.now);
    }
}

fn paragraphs_of<'a>(block: &'a Block, out: &mut Vec<&'a Arc<Paragraph>>) {
    match block {
        Block::Paragraph(p) => out.push(p),
        Block::Table(t) => {
            for cell in t.rows.iter().flat_map(|r| r.iter()) {
                cell.iter().for_each(|b| paragraphs_of(b, out));
            }
        }
    }
}

impl Model {
    fn block_items(&mut self, cx: &Context, top: i64, block: &Block) -> BlockItems {
        let mut paragraphs = Vec::new();
        paragraphs_of(block, &mut paragraphs);
        let items = paragraphs
            .into_iter()
            .map(|p| {
                let item = item_of(&self.resolver.paragraph_props(cx, p));
                (item, item.and_then(|i| item_lists(&cx.numbering, i, top)))
            })
            .collect();
        BlockItems::new(items)
    }

    fn items_of(&mut self, cx: &Context, top: i64, key: Digest, block: &Block) -> BlockItems {
        if let Some(found) = self.label_memo.get(&key) {
            return found;
        }
        let made = self.block_items(cx, top, block);
        self.label_memo.insert(key, made.clone());
        made
    }

    /// Bring the body's labels up to date: the blocks between the unchanged ones at either
    /// end are taken out and the new ones put in.
    pub(super) fn update_labels(&mut self, cx: &Context, body: &[Block]) {
        let keys: Vec<Digest> = body.iter().map(|b| self.block_keys.of(self.key, b)).collect();
        self.block_keys.finish();
        let old = std::mem::take(&mut self.label_keys);
        let prefix = old.iter().zip(&keys).take_while(|(a, b)| a == b).count();
        let rest = old.len().min(keys.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(keys.iter().rev())
            .take(rest)
            .take_while(|(a, b)| a == b)
            .count();
        let (removed, added) = (old.len() - prefix - suffix, keys.len() - prefix - suffix);
        let top = top_level(&cx.numbering);
        let mut edits = 0;
        if old.is_empty() {
            let mut made = Vec::with_capacity(body.len());
            for (block, key) in body.iter().zip(&keys) {
                made.push(self.items_of(cx, top, *key, block));
            }
            self.labels = made.into_iter().collect();
            self.stats.label_blocks = body.len();
            self.stats.label_edits = body.len();
            self.label_keys = keys;
            return;
        }
        for _ in 0..removed {
            if self.labels.remove(prefix).is_ok() {
                edits += 1;
            }
        }
        for (k, block) in body.iter().enumerate().skip(prefix).take(added) {
            let key = keys.get(k).copied().unwrap_or_default();
            let items = self.items_of(cx, top, key, block);
            if self.labels.insert(k, items).is_ok() {
                edits += 1;
            }
        }
        self.stats.label_blocks = added;
        self.stats.label_edits = edits;
        self.label_keys = keys;
    }
}

impl Model {
    fn label_of(&mut self, cx: &Context, counters: &mut Counters, p: &Paragraph) -> Option<String> {
        let item = item_of(&self.resolver.paragraph_props(cx, p))?;
        counters.item(&cx.numbering, item.num_id, item.ilvl)
    }

    /// The list labels of a story's paragraphs (a header's, a footer's, a note's), in
    /// document order with cells, counted from scratch: one counter per story, as layout
    /// counts a story of its own.
    pub fn story_labels(&mut self, document: &WordDocument, blocks: &[Block]) -> Vec<Option<String>> {
        let mut counters = Counters::default();
        let mut paragraphs = Vec::new();
        for block in blocks {
            paragraphs_of(block, &mut paragraphs);
        }
        paragraphs
            .into_iter()
            .map(|p| self.label_of(&document.context, &mut counters, p))
            .collect()
    }

    /// The list labels in the body's text boxes, by the path their paragraph has once laid
    /// out in its box (the anchor paragraph's path, then its own; a cell's paragraph its
    /// own): the text boxes are one story, their lists continuing from box to box in the
    /// order of their anchors (docx2svg's `text_box_labels`, measured).
    pub fn text_box_labels(&mut self, document: &WordDocument) -> BTreeMap<Text, Option<String>> {
        let mut boxes = TextBoxes {
            model: self,
            cx: &document.context,
            counters: Counters::default(),
            out: BTreeMap::new(),
        };
        boxes.paragraphs(&document.body);
        boxes.out
    }
}

/// The walk of the body's text boxes for their labels.
struct TextBoxes<'m> {
    model: &'m mut Model,
    cx: &'m Context,
    counters: Counters,
    out: BTreeMap<Text, Option<String>>,
}

impl TextBoxes<'_> {
    fn blocks(&mut self, items: &[Block], prefix: Option<&str>) {
        for item in items {
            match item {
                Block::Table(t) => t
                    .rows
                    .iter()
                    .flat_map(|r| r.iter())
                    .for_each(|cell| self.blocks(cell, None)),
                Block::Paragraph(p) => {
                    let path = prefix.map_or_else(|| p.path.clone(), |pre| text(&format!("{pre}/{}", p.path)));
                    let label = self.model.label_of(self.cx, &mut self.counters, p);
                    self.out.insert(path, label);
                }
            }
        }
    }

    fn graphic(&mut self, found: &crate::model::Graphic, anchor_path: &str) {
        if !found.text.is_empty() {
            self.blocks(&found.text, Some(anchor_path));
        }
        for inner in found.children.iter() {
            self.graphic(inner, anchor_path);
        }
    }

    fn paragraphs(&mut self, items: &[Block]) {
        for item in items {
            match item {
                Block::Table(t) => t
                    .rows
                    .iter()
                    .flat_map(|r| r.iter())
                    .for_each(|cell| self.paragraphs(cell)),
                Block::Paragraph(p) => {
                    for run in p.runs.iter() {
                        for found in run.drawings.iter() {
                            if let crate::model::InlineObject::Graphic(g) = found {
                                self.graphic(g, &p.path);
                            }
                        }
                        for anchor in run.anchors.iter() {
                            if let Some(g) = &anchor.graphic {
                                self.graphic(g, &p.path);
                            }
                        }
                    }
                }
            }
        }
    }
}
