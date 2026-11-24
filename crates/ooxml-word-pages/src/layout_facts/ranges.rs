//! Owns: what `check`'s `range` says of each range from the layout (docx-agent's
//! `scoped.range_extent`, `range_keeps`, round 35 R26, round 36 R30): how much of each page
//! its body content takes ([`Facts::extent`], `DocumentLayout.extent`), where a range of
//! whole blocks breaks over pages ([`Facts::range_layout`]: between two blocks, or inside
//! one, and whether the later block starts a page anyway) and whether a kept range starts a
//! page only because of its keep ([`Facts::starts_page_by_keep`], `keep_trials._wanted`).
//!
//! Must not: say whether a range is kept or what fixes it (the tool layer reads the keeps
//! a call would write), or cap a list.

use super::tree::{block_keys_top, is_w, page_break_at, paragraphs_below, BlockKind, W};
use super::{points, Block, Facts, Range};
use ooxml_word_layout::facts::PageFacts;
use ooxml_word_layout::px::Px;
use ooxml_word_model::docx::at_path;
use ooxml_word_model::docx::sections::sections;
use ooxml_word_model::ids::Entry;
use ooxml_word_model::xmlread::py_int;
use ooxml_xml::Element;
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

/// How much of one page some blocks' body content takes (`layout.Extent`).
#[derive(Debug, Clone, PartialEq)]
pub struct Extent {
    /// The 1-based page.
    pub page: usize,
    /// The top of their first line there, points from the page's top edge.
    pub top: f64,
    /// The bottom of their last line there (its top plus its pitch).
    pub bottom: f64,
    /// How many of their lines it draws.
    pub lines: usize,
    /// How many text columns they are drawn in.
    pub columns: usize,
    /// The page's height less its section's top and bottom margins, points.
    pub body: f64,
    /// Where the body is laid out on the page, `(top, bottom)` in points.
    pub area: (f64, f64),
}

/// What of some blocks a text box may draw ([`Facts::extent`]).
#[derive(Debug, Default)]
struct Boxed {
    /// The place of each text box's paragraph (`false`) or table (`true`) among them.
    places: Vec<(Vec<u32>, bool)>,
    /// The 1-based pages the blocks holding them are drawn on.
    pages: BTreeSet<usize>,
}

/// A page break in a range of whole blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Break {
    /// Between two blocks: the later starts on `page`; `page_break` where it starts a page
    /// anyway (a page break, a section start), which no keep undoes.
    Between {
        /// The earlier block's id.
        after: String,
        /// The later block's id.
        before: String,
        /// The 1-based page the later starts on.
        page: usize,
        /// Whether it starts a page whatever keeps it.
        page_break: bool,
    },
    /// Within one block, drawn on `pages`.
    Inside {
        /// The block's id.
        block: String,
        /// Its 1-based pages.
        pages: Vec<usize>,
    },
}

/// Where a range of whole body blocks is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct RangeLayout {
    /// Whether it is drawn on one page.
    pub one_page: bool,
    /// Its page breaks, in order (none on one page).
    pub breaks: Vec<Break>,
    /// The height of its body content, summed over its pages, points.
    pub height: f64,
    /// The least room a page it is on gives the body (`area`'s height), points.
    pub room: f64,
}

impl Facts<'_> {
    /// How much of each page the body content of the paragraphs and tables `ids` takes,
    /// one [`Extent`] per page any of them draws a line on, in page order, on the 1-based
    /// `pages` (`None`: every page): a table by its cells' lines, a text box's paragraph by
    /// the lines the box draws (a table's boxes too; a paragraph's own lines only, not its
    /// boxes': `DocumentLayout.extent` reads every line docx2svg draws, by its id).
    #[must_use]
    pub fn extent(&self, ids: &[String], pages: Option<&[usize]>) -> Vec<Extent> {
        let version = self.version();
        let wanted: HashSet<String> = ids
            .iter()
            .filter_map(|id| version.path_of(id))
            .map(|p| p.to_string())
            .collect();
        let held =
            |path: &str| wanted.contains(path) || super::tree::table_paths(path).any(|table| wanted.contains(table));
        let all = &version.facts().pages;
        let numbers: BTreeSet<usize> = match pages {
            None => (1..=all.len()).collect(),
            Some(pages) => pages.iter().copied().filter(|n| (1..=all.len()).contains(n)).collect(),
        };
        let bodies = section_bodies(self);
        let boxed = self.boxed(ids);
        let mut out = Vec::new();
        for number in numbers {
            let Some(page) = all.get(number - 1) else { continue };
            let mut mine: Vec<(Px, Px, usize)> = page
                .lines
                .iter()
                .filter(|l| super::story_kind(&l.path) == "body" && version.is_named(&l.path))
                .filter(|l| held(&l.path))
                .map(|l| (l.top, l.pitch, l.column.unwrap_or(0)))
                .collect();
            if boxed.pages.contains(&number) {
                mine.extend(self.box_lines(&boxed, number));
            }
            let (Some(top), Some(bottom)) = (mine.iter().map(|l| l.0).min(), mine.iter().map(|l| l.0 + l.1).max())
            else {
                continue;
            };
            let columns: HashSet<usize> = mine.iter().map(|l| l.2).collect();
            let (body, area) = body_of(page, &bodies);
            out.push(Extent {
                page: number,
                top: points(top),
                bottom: points(bottom),
                lines: mine.len(),
                columns: columns.len(),
                body,
                area,
            });
        }
        out
    }

    /// What of `ids` a text box may draw: the places of those that are a text box's
    /// paragraph, of the tables among them (a cell's boxes are the table's), and the
    /// 1-based pages the blocks holding them are drawn on (only they are painted for it).
    fn boxed(&self, ids: &[String]) -> Boxed {
        let mut boxed = Boxed::default();
        let Some(body) = self.docx().body() else {
            return boxed;
        };
        let version = self.version();
        for id in ids {
            let (place, held_by, table) = match body.ids.get(id) {
                Some(Entry::Paragraph(_)) => match version.ids.in_paragraph(id) {
                    Some((host, place)) => (place, host.to_string(), false),
                    None => continue,
                },
                Some(Entry::Table(k)) => match (body.ids.tables().get(k), version.path_of(id)) {
                    (Some(table), Some(path)) => (table.path.to_vec(), path.to_string(), true),
                    _ => continue,
                },
                None => continue,
            };
            let top = super::tree::top_block_path(&held_by).unwrap_or(&held_by);
            boxed.pages.extend(self.block_pages(top));
            boxed.places.push((place, table));
        }
        boxed
    }

    /// `(top, pitch, column)` of each line the body's text boxes draw on the 1-based page
    /// `number` of the paragraphs (or inside the tables) `boxed` holds.
    fn box_lines(&self, boxed: &Boxed, number: usize) -> Vec<(Px, Px, usize)> {
        let version = self.version();
        self.read(number - 1)
            .map(|read| {
                read.box_texts
                    .iter()
                    .filter(|l| l.story.is_none() && super::story_kind(&l.path) == "body")
                    .filter(|l| {
                        version.ids.place_of_path(&l.path).is_some_and(|at| {
                            boxed
                                .places
                                .iter()
                                .any(|(place, table)| if *table { at.starts_with(place) } else { at == *place })
                        })
                    })
                    .map(|l| (l.top, l.pitch, l.column.unwrap_or(0)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Where the range `range` of whole blocks -- `blocks`, the ids of the body's blocks
    /// its address names (a control's blocks in its place; a paragraph of a cell, named
    /// alone, is its own block) -- is drawn: `None` where it is not whole blocks (a cell, a
    /// text range: a table holds a paragraph the range does not) or not drawn whole.
    #[must_use]
    pub fn range_layout(&self, range: &Range, blocks: &[String]) -> Option<RangeLayout> {
        let held: HashSet<&str> = range.paragraphs.iter().map(String::as_str).collect();
        let body = self.docx().body()?;
        let mut owned: Vec<Block<'_>> = Vec::new();
        for id in blocks {
            let block = self.body_block(id)?;
            // A paragraph is whole where it is held (what its text boxes hold is its own); a
            // table where every paragraph in it is (`range_keeps`).
            let whole = match block.kind {
                BlockKind::Paragraph => held.contains(block.id.as_str()),
                BlockKind::Table => paragraphs_below(block.element, &block.place).iter().all(|(_, place)| {
                    body.ids
                        .paragraph_at(place)
                        .is_some_and(|p| held.contains(p.id.as_str()))
                }),
            };
            if !whole {
                return None;
            }
            owned.push(block);
        }
        let found: Vec<&Block<'_>> = owned.iter().collect();
        let drawn: Vec<Vec<usize>> = found.iter().map(|b| self.block_pages(&b.path)).collect();
        if found.is_empty() || drawn.iter().any(Vec::is_empty) {
            return None;
        }
        let every: BTreeSet<usize> = drawn.iter().flatten().copied().collect();
        let one_page = every.len() == 1;
        let mut breaks = Vec::new();
        if !one_page {
            for (k, (block, pages)) in found.iter().zip(&drawn).enumerate() {
                let earlier = k.checked_sub(1).and_then(|j| Some((found.get(j)?, drawn.get(j)?)));
                if let Some((before, before_pages)) = earlier {
                    let (last, first) = (before_pages.iter().max(), pages.iter().min());
                    if let (Some(&last), Some(&first)) = (last, first) {
                        if last < first {
                            breaks.push(Break::Between {
                                after: before.id.clone(),
                                before: block.id.clone(),
                                page: first,
                                page_break: self.starts_page(before.element, block.element),
                            });
                        }
                    }
                }
                if pages.len() > 1 {
                    breaks.push(Break::Inside {
                        block: block.id.clone(),
                        pages: pages.clone(),
                    });
                }
            }
        }
        let extents = self.extent(&range.paragraphs, None);
        let height = extents.iter().map(|e| e.bottom - e.top).sum();
        let room = extents
            .iter()
            .map(|e| e.area.1 - e.area.0)
            .reduce(f64::min)
            .unwrap_or(0.0);
        Some(RangeLayout {
            one_page,
            breaks,
            height,
            room,
        })
    }

    /// The body's block `id` names -- a top-level one, or a paragraph or table inside one
    /// (a cell's) -- with its place and layout path.
    fn body_block(&self, id: &str) -> Option<Block<'_>> {
        if let Some(top) = self.blocks().iter().find(|b| b.id == id) {
            return Some(top.clone());
        }
        let body = self.docx().body()?;
        let (kind, place) = match body.ids.get(id)? {
            Entry::Paragraph(k) => (BlockKind::Paragraph, body.ids.paragraphs().get(k)?.path),
            Entry::Table(k) => (BlockKind::Table, body.ids.tables().get(k)?.path),
        };
        Some(Block {
            kind,
            element: at_path(body.root(), &place)?,
            place: place.to_vec(),
            path: self.version().path_of(id)?.to_string(),
            id: id.to_owned(),
        })
    }

    /// Whether a range kept on one page, starting on the 1-based `start`, starts that page
    /// only because of its keep: its first block is the first top-level block drawn there,
    /// on a page after the first, and the block before it neither breaks the page nor
    /// ends a section.
    #[must_use]
    pub fn starts_page_by_keep(&self, range: &Range, start: usize) -> bool {
        let Some(first) = range.blocks.first() else {
            return false;
        };
        if start < 2 || self.blocks_on(start).first().is_none_or(|b| b.id != *first) {
            return false;
        }
        let Some(block) = self.blocks().iter().find(|b| b.id == *first) else {
            return false;
        };
        match self.block_before(block) {
            Some(before) => !self.starts_page(&before, block.element),
            None => false,
        }
    }

    /// The 1-based pages that draw something of the body block at layout path `path` (its
    /// key among a page's blocks: `pages_of`).
    #[must_use]
    pub fn block_pages(&self, path: &str) -> Vec<usize> {
        self.version()
            .facts()
            .pages
            .iter()
            .enumerate()
            .filter(|(_, p)| p.blocks.iter().any(|b| **b == *path))
            .map(|(k, _)| k + 1)
            .collect()
    }

    /// The top-level blocks the 1-based `page` draws something of, in document order
    /// (`blocks_on`).
    #[must_use]
    pub fn blocks_on(&self, page: usize) -> Vec<&Block<'_>> {
        let Some(facts) = page.checked_sub(1).and_then(|k| self.version().facts().pages.get(k)) else {
            return Vec::new();
        };
        let mut found: Vec<(usize, &Block<'_>)> = facts
            .blocks
            .iter()
            .filter(|key| block_keys_top(key))
            .filter_map(|key| self.block_at(key))
            .collect();
        found.sort_by_key(|(k, _)| *k);
        found.dedup_by_key(|(k, _)| *k);
        found.into_iter().map(|(_, b)| b).collect()
    }

    /// Whether `element` starts a page whatever keeps it (`_starts_page`): a page break
    /// before it (its style's or its own), a page break at its start or at the end of the
    /// paragraph `before` it, or a section break there.
    #[must_use]
    pub fn starts_page(&self, before: &Arc<Element>, element: &Arc<Element>) -> bool {
        if is_w(element, "p") && (self.paragraph(element).page_break_before || page_break_at(element, true)) {
            return true;
        }
        is_w(before, "p")
            && (page_break_at(before, false)
                || before
                    .find(Some(W), "pPr")
                    .is_some_and(|p| p.find(Some(W), "sectPr").is_some()))
    }

    /// The paragraph or table just before `block` among its siblings (`_block_before`).
    fn block_before(&self, block: &Block<'_>) -> Option<Arc<Element>> {
        let (&last, above) = block.place.split_last()?;
        let parent = at_path(self.docx().body()?.root(), above)?;
        parent
            .children()
            .iter()
            .take(last as usize)
            .filter_map(|n| n.as_element())
            .filter(|e| is_w(e, "p") || is_w(e, "tbl"))
            .last()
            .cloned()
    }
}

/// Each section's page height less its top and bottom margins, points (`None` where its
/// `w:sectPr` states no page height or margins), in document order (`_section_bodies`).
fn section_bodies(facts: &Facts<'_>) -> Vec<Option<f64>> {
    sections(facts.docx())
        .iter()
        .map(|section| {
            let properties = &section.properties;
            let int = |local: &str, attribute: &str| {
                properties
                    .find(Some(W), local)
                    .and_then(|e| e.attribute(Some(W), attribute))
                    .and_then(py_int)
            };
            let (height, top, bottom) = (int("pgSz", "h")?, int("pgMar", "top")?, int("pgMar", "bottom")?);
            #[allow(clippy::cast_precision_loss)]
            let body = (height - top.abs() - bottom.abs()) as f64 / 20.0;
            Some(super::round(body, 2))
        })
        .collect()
}

/// `(body, area)` of a page ([`Extent`]).
fn body_of(page: &PageFacts, bodies: &[Option<f64>]) -> (f64, (f64, f64)) {
    let area = (points(page.frame.top), points(page.frame.bottom));
    let body = bodies
        .get(page.info.section)
        .copied()
        .flatten()
        .unwrap_or_else(|| super::round(area.1 - area.0, 2));
    (body, area)
}

/// What a page draws that its facts leave out, as the blank at its foot is measured
/// (`PageEngine::beyond_facts`): lines docx-agent's `Page.text_lines` counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BeyondFacts {
    /// The bottom of the lowest line its body's text boxes draw.
    pub text_boxes: Option<Px>,
    /// The top of its footnotes' first line (their separator's).
    pub notes: Option<Px>,
}

/// The blank at the foot of a page's body (`keep_trials._room`): from the bottom of its
/// last body line (its text boxes' included, `beyond`) to the bottom of where the body may
/// be drawn (or to the first note line drawn above it), in points (one place) and as a
/// share of that height (two places).
#[must_use]
pub fn room(page: &PageFacts, beyond: &BeyondFacts) -> (f64, f64) {
    let top = page.frame.top;
    let mut bottom = beyond
        .notes
        .map_or(page.frame.bottom, |notes| page.frame.bottom.min(notes));
    let mut lowest = beyond.text_boxes.map_or(top, |foot| top.max(foot));
    for line in &page.lines {
        if super::story_kind(&line.path) == "body" {
            lowest = lowest.max(line.top + line.pitch);
        } else if line.path.starts_with("w:footnote") || line.path.starts_with("w:endnote") {
            bottom = bottom.min(line.top);
        }
    }
    let height = bottom - top;
    let blank = (bottom - lowest).max(Px::int(0));
    let pt = super::round(super::ratio(blank.value() / Px::per_point()), 1);
    let share = if height > Px::int(0) {
        super::round(super::ratio(blank.value() / height.value()), 2)
    } else {
        0.0
    };
    (pt, share)
}
