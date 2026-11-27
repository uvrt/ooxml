//! Owns: where `word_inspect`'s blocks are placed when the session lays the version out
//! itself ([`Laid`]): each block's placements from the version's layout, as docx-agent's
//! `DocumentLayout.where` reads a Rust layout -- a body paragraph's from the flow's lines,
//! a table's from its paragraphs' lines, a header's, footer's, note's or text box's
//! paragraph from the lines every page paints (painted once, when such a block is first
//! asked for) -- grouped by page and text column, and `unknown` (`not-drawn`) for a block
//! nothing of which is drawn.
//!
//! Must not: lay out but through the call's layout, or answer in other words than a
//! [`crate::Placer`]'s (`placements` or `unknown`, as docx-agent's `state` writes them).

use crate::pages::{NotLaid, WordLayout};
use ooxml_base::Cancel;
use ooxml_word_layout::px::Px;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::{BlockKey, StoryIx, Sub};
use ooxml_word_model::WordPackage;
use ooxml_word_pages::layout_facts::points;
use ooxml_word_pages::line_text::ItemPlace;
use ooxml_word_pages::LaidVersion;
use serde_json::{json, Map, Value};
use std::cell::OnceCell;
use std::collections::HashMap;
use std::sync::Arc;

/// One line drawn, as a block's placements read it.
#[derive(Debug, Clone, Copy)]
struct Line {
    /// The 0-based page.
    page: usize,
    /// Its place among the lines the page draws.
    place: usize,
    top: Px,
    pitch: Px,
    column: usize,
    story: &'static str,
}

/// A version laid out by the call's layout, asked where blocks are placed.
pub(super) struct Laid<'v> {
    version: Arc<LaidVersion>,
    layout: &'v dyn WordLayout,
    docx: &'v Docx,
    /// Each body paragraph's lines, and each table's and row's (its paragraphs'), by key,
    /// in drawing order.
    body: HashMap<BlockKey, Vec<Line>>,
    /// Each paragraph's lines among every line the pages paint (headers, footers, notes,
    /// text boxes), by id: read when a block without body lines is first asked for.
    painted: OnceCell<HashMap<String, Vec<Line>>>,
}

impl<'v> Laid<'v> {
    /// `package` (read as `docx`) laid out by `layout`, stopped where `cancel` says so.
    pub(super) fn new(
        layout: &'v dyn WordLayout,
        package: &WordPackage,
        docx: &'v Docx,
        cancel: &dyn Cancel,
    ) -> Result<Laid<'v>, NotLaid> {
        let version = layout.laid(package, cancel)?;
        let names = &version.layout().document.names;
        let mut body: HashMap<BlockKey, Vec<Line>> = HashMap::new();
        // the blocks a paragraph's lines count for: itself, and the tables and rows it is in
        let mut blocks: HashMap<BlockKey, Vec<BlockKey>> = HashMap::new();
        for (page, facts) in version.facts().pages.iter().enumerate() {
            for (place, line) in facts.lines.iter().enumerate() {
                let Some(key) = line.anchor.map(|a| a.block).filter(|k| k.story() == StoryIx::BODY) else {
                    continue;
                };
                let held = blocks.entry(key).or_insert_with(|| {
                    let chain = names.blocks_of(key).unwrap_or_default();
                    let above = chain.iter().filter(|a| a.sub == Sub::Whole && a.block != key);
                    std::iter::once(key).chain(above.map(|a| a.block)).collect()
                });
                let drawn = Line {
                    page,
                    place,
                    top: line.top,
                    pitch: line.pitch,
                    column: line.column.unwrap_or(0),
                    story: "body",
                };
                for block in held.iter() {
                    body.entry(*block).or_default().push(drawn);
                }
            }
        }
        Ok(Laid {
            version,
            layout,
            docx,
            body,
            painted: OnceCell::new(),
        })
    }

    /// For each entry of `asked` (an id, then the ids it joins, tried in order while
    /// nothing is drawn of those before), its placements' keys (`placements.rs`).
    pub(super) fn place(&self, asked: &[Vec<String>]) -> Vec<Map<String, Value>> {
        asked
            .iter()
            .map(|ids| {
                let mut found = Vec::new();
                for id in ids {
                    found = self.lines_of(id);
                    if !found.is_empty() {
                        break;
                    }
                }
                keys(&found)
            })
            .collect()
    }

    /// The lines the block `id` is drawn with, in drawing order: a table's, its
    /// paragraphs'.
    fn lines_of(&self, id: &str) -> Vec<Line> {
        let names = &self.version.layout().document.names;
        if let Some(key) = names.key_of_id(StoryIx::BODY, id) {
            let mut lines: Vec<Line> = self.body.get(&key).cloned().unwrap_or_default();
            if !lines.is_empty() {
                // drawing order: by page, then by place among the page's lines
                lines.sort_by_key(|line| (line.page, line.place));
                return lines;
            }
        }
        self.painted().get(id).cloned().unwrap_or_default()
    }

    /// Every line the pages paint, by its paragraph's id, read once.
    fn painted(&self) -> &HashMap<String, Vec<Line>> {
        self.painted.get_or_init(|| {
            let mut named: HashMap<(Option<String>, ItemPlace), Option<String>> = HashMap::new();
            let mut out: HashMap<String, Vec<Line>> = HashMap::new();
            let pages = self.version.facts().pages.len();
            for page in 0..pages {
                let read = {
                    let mut engine = ooxml_word_pages::lock_engine(self.layout.engine());
                    engine.page_read(&self.version, page)
                };
                let Some(read) = read else {
                    continue;
                };
                let read = read.read();
                for (place, line) in read.texts.iter().chain(&read.box_texts).enumerate() {
                    let part = line.story.as_ref().map(|(_, part)| part.clone());
                    let id = named
                        .entry((part, line.place.clone()))
                        .or_insert_with(|| self.version.line_paragraph_id(self.docx, line));
                    if let Some(id) = id {
                        out.entry(id.clone()).or_default().push(Line {
                            page,
                            place,
                            top: line.top,
                            pitch: line.pitch,
                            column: line.column.unwrap_or(0),
                            story: line.at.story,
                        });
                    }
                }
            }
            out
        })
    }
}

/// What a block says of where it is placed: one placement per page and text column, in
/// page order and, on a page, in the order its columns are first drawn; `unknown` where
/// nothing of it is drawn.
fn keys(lines: &[Line]) -> Map<String, Value> {
    let mut out = Map::new();
    if lines.is_empty() {
        out.insert("unknown".into(), json!({"reason": "not-drawn", "after_page": null}));
        return out;
    }
    let mut placements: Vec<(usize, usize, Px, Px, usize, &'static str)> = Vec::new();
    let mut first_of_page = 0;
    let mut page = None;
    for line in lines {
        if page != Some(line.page) {
            page = Some(line.page);
            first_of_page = placements.len();
        }
        let bottom = line.top + line.pitch;
        let mine = placements
            .get_mut(first_of_page..)
            .and_then(|on| on.iter_mut().find(|p| p.1 == line.column));
        match mine {
            Some(p) => {
                p.2 = p.2.min(line.top);
                p.3 = p.3.max(bottom);
                p.4 += 1;
            }
            None => placements.push((line.page, line.column, line.top, bottom, 1, line.story)),
        }
    }
    let placements: Vec<Value> = placements
        .into_iter()
        .map(|(page, column, top, bottom, lines, story)| {
            json!({"page": page + 1, "top": points(top), "bottom": points(bottom), "lines": lines,
                   "story": story, "column": column})
        })
        .collect();
    out.insert("placements".into(), Value::Array(placements));
    out
}
