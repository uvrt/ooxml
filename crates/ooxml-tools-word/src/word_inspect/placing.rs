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
use ooxml_opc::Package;
use ooxml_word_layout::px::Px;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::read::paths::resolve_path;
use ooxml_word_pages::layout_facts::{points, story_kind};
use ooxml_word_pages::line_text::LineText;
use ooxml_word_pages::LaidVersion;
use serde_json::{json, Map, Value};
use std::cell::OnceCell;
use std::collections::HashMap;
use std::sync::{Arc, PoisonError};

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
    /// Each body paragraph's lines, by path, in drawing order.
    body: HashMap<String, Vec<Line>>,
    /// Each paragraph's lines among every line the pages paint (headers, footers, notes,
    /// text boxes), by id: read when a block without body lines is first asked for.
    painted: OnceCell<HashMap<String, Vec<Line>>>,
}

impl<'v> Laid<'v> {
    /// `package` (read as `docx`) laid out by `layout`, stopped where `cancel` says so.
    pub(super) fn new(
        layout: &'v dyn WordLayout,
        package: &Package,
        docx: &'v Docx,
        cancel: &dyn Cancel,
    ) -> Result<Laid<'v>, NotLaid> {
        let version = layout.laid(package, cancel)?;
        let mut body: HashMap<String, Vec<Line>> = HashMap::new();
        for (page, facts) in version.facts().pages.iter().enumerate() {
            for (place, line) in facts.lines.iter().enumerate() {
                if story_kind(&line.path) != "body" {
                    continue;
                }
                body.entry(line.path.to_string()).or_default().push(Line {
                    page,
                    place,
                    top: line.top,
                    pitch: line.pitch,
                    column: line.column.unwrap_or(0),
                    story: "body",
                });
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
        if let Some(path) = self.version.path_of(id) {
            let path: &str = &path;
            let inside = format!("{path}/");
            let mut lines: Vec<Line> = self
                .body
                .iter()
                .filter(|(p, _)| p.as_str() == path || p.starts_with(&inside))
                .flat_map(|(_, lines)| lines.iter().copied())
                .collect();
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
            let mut named: HashMap<(String, String), Option<String>> = HashMap::new();
            let mut out: HashMap<String, Vec<Line>> = HashMap::new();
            let pages = self.version.facts().pages.len();
            for page in 0..pages {
                let read = {
                    let mut engine = self.layout.engine().lock().unwrap_or_else(PoisonError::into_inner);
                    engine.page_read(&self.version, page)
                };
                let Some(read) = read else {
                    continue;
                };
                for (place, line) in read.texts.iter().chain(&read.box_texts).enumerate() {
                    let Some(part) = self.part_of(line) else {
                        continue;
                    };
                    let id = named
                        .entry((part.clone(), line.path.clone()))
                        .or_insert_with(|| self.id_at(&part, &line.path));
                    if let Some(id) = id {
                        out.entry(id.clone()).or_default().push(Line {
                            page,
                            place,
                            top: line.top,
                            pitch: line.pitch,
                            column: line.column.unwrap_or(0),
                            story: line
                                .story
                                .as_ref()
                                .map_or_else(|| story_kind(&line.path), |(kind, _)| kind),
                        });
                    }
                }
            }
            out
        })
    }

    /// The part a painted line's paragraph is in: its story's, the notes', else the main
    /// part's.
    fn part_of(&self, line: &LineText) -> Option<String> {
        if let Some((_, part)) = &line.story {
            return Some(part.clone());
        }
        match story_kind(&line.path) {
            kind @ ("footnotes" | "endnotes") => self.docx.story(kind).map(|s| s.part.clone()),
            _ => self.docx.body().map(|s| s.part.clone()),
        }
    }

    /// The id of the paragraph at `path` of the part `part`.
    fn id_at(&self, part: &str, path: &str) -> Option<String> {
        let story = self.docx.part(part)?;
        let (_, place) = resolve_path(story.root(), path)?;
        story.ids.paragraph_at(&place).map(|p| p.id)
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
