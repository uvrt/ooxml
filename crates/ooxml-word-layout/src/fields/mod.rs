//! Owns: the body's page-number fields settled against the layout (docx2svg's
//! `_Placer._body_field_values`, `_page_of`, `_bookmark_pages`, `_page_count`,
//! `_section_pages`, `substitute_document`): which runs hold a `PAGE`, `NUMPAGES`,
//! `SECTIONPAGES` or `PAGEREF` ([`page_fields`]), what each draws on the page its line is
//! on ([`field_values`]), and the document with those values in its runs
//! ([`substitute`]). The engine lays the body out again with the values found until they
//! are the ones it was laid out with.
//!
//! `SEQ` and `REF`, which do not depend on the pages, are computed once before the passes
//! (`references`).
//!
//! `STYLEREF` is computed in the body once before the passes, and in headers and footers
//! per page (`styleref`).
//!
//! Must not: draw a field.

mod format;
mod references;
pub mod styleref;

pub use format::{argument, field_text, format_value, number_text, switches, Uncomputable};
pub use references::{cross_references, Computed};

use crate::engine::DocumentLayout;
use crate::flow::{body_paragraphs, FlowItem};
use ooxml_text::PieceKind;
use ooxml_word_model::model::{Block, Paragraph, Run, Table};
use ooxml_word_model::WordDocument;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// The fields whose value is a page number.
pub const PAGE_FIELDS: [&str; 4] = ["PAGE", "NUMPAGES", "SECTIONPAGES", "PAGEREF"];

/// A run holding a computed field: its paragraph (the body's paragraphs in document
/// order, cells included) and its index in the paragraph.
pub type FieldKey = (usize, usize);

/// Each page-number field's text.
pub type FieldValues = BTreeMap<FieldKey, String>;

/// Every run of the body holding a page-number field.
pub fn page_fields(document: &WordDocument) -> Vec<FieldKey> {
    let (paragraphs, _) = body_paragraphs(&document.body);
    let mut out = Vec::new();
    for (block, paragraph) in paragraphs.iter().enumerate() {
        for (k, run) in paragraph.runs.iter().enumerate() {
            if run.field.as_ref().is_some_and(|f| PAGE_FIELDS.contains(&&*f.keyword)) {
                out.push((block, k));
            }
        }
    }
    out
}

/// Where the body's paragraphs' lines are: per flow item asked about, the page of each
/// line.
struct Pages<'a> {
    layout: &'a DocumentLayout,
    lines: HashMap<usize, Vec<usize>>,
    /// A table cell's paragraph -> the page of its first line drawn.
    cells: HashMap<usize, usize>,
}

/// The flow item of paragraph `block` (the body's, cells included), where it is a
/// top-level paragraph: items stand in block order.
fn item_of_block(items: &[FlowItem], block: usize) -> Option<usize> {
    let k = items.partition_point(|i| i.block() < block);
    match items.get(k) {
        Some(FlowItem::Para(p)) if p.block == block => Some(k),
        _ => None,
    }
}

/// Each table cell paragraph's first page: the page its part of its row is on, for each
/// line it draws (`paginate.drawn_pages`).
fn cell_pages(layout: &DocumentLayout) -> HashMap<usize, usize> {
    let mut out = HashMap::new();
    for_cell_lines(layout, &mut |block, page| {
        out.entry(block).or_insert(page);
    });
    out
}

/// Every drawn line of a table cell's paragraph, as its block (the body's, cells
/// included) and the page its part of its row is on, in page order.
fn for_cell_lines(layout: &DocumentLayout, each: &mut dyn FnMut(usize, usize)) {
    let items = &layout.flow.items;
    for page in &layout.pagination.pages {
        let (start, end) = (&page.start, &page.end);
        for index in start.item..(end.item + 1).min(items.len()) {
            let Some(FlowItem::Table(t)) = items.get(index) else {
                continue;
            };
            let rows = t.flow.rows();
            let first_row = if index == start.item { start.line } else { 0 };
            let last_row = if index == end.item { end.line } else { rows };
            for r in first_row..(last_row + 1).min(rows) {
                for (c, stack) in t.flow.stacks(r).iter().enumerate() {
                    let at = |p: &[u32]| p.get(c).map_or(0, |v| usize::try_from(*v).unwrap_or(0));
                    let lo = if index == start.item && r == start.line && !start.part.is_empty() {
                        at(&start.part)
                    } else {
                        0
                    };
                    let hi = match (index == end.item && r == end.line, end.part.is_empty()) {
                        (true, false) => at(&end.part),
                        (true, true) => 0,
                        (false, _) => stack.len(),
                    };
                    for block in stack.get(lo..hi).unwrap_or_default().iter().flatten() {
                        each(t.block + block, page.page);
                    }
                }
            }
        }
    }
}

impl<'a> Pages<'a> {
    /// The pages of the paragraphs `blocks` (the body's, cells included; `None`: every
    /// one).
    fn new(layout: &'a DocumentLayout, blocks: Option<&BTreeSet<usize>>) -> Pages<'a> {
        let items = &layout.flow.items;
        let wanted: Option<Vec<bool>> = blocks.map(|blocks| {
            let mut wanted = vec![false; items.len()];
            for block in blocks {
                if let Some(k) = item_of_block(items, *block) {
                    if let Some(w) = wanted.get_mut(k) {
                        *w = true;
                    }
                }
            }
            wanted
        });
        let asked = |item: usize| wanted.as_ref().is_none_or(|w| w.get(item).copied().unwrap_or(false));
        let mut lines: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
        for page in &layout.pagination.pages {
            for line in page.lines.iter().filter(|l| asked(l.item)) {
                lines.entry(line.item).or_default().push((line.line, page.page));
            }
        }
        let lines = lines
            .into_iter()
            .map(|(item, mut found)| {
                found.sort_unstable();
                found.dedup_by_key(|(line, _)| *line);
                (item, found.into_iter().map(|(_, page)| page).collect())
            })
            .collect();
        // A table cell's paragraphs are looked up on the pages only where one is asked for.
        let in_cells = blocks.is_none_or(|b| b.iter().any(|block| item_of_block(items, *block).is_none()));
        Pages {
            cells: if in_cells { cell_pages(layout) } else { HashMap::new() },
            layout,
            lines,
        }
    }

    /// The page a body paragraph's character `offset` is drawn on (`_page_of`).
    fn page_of(&self, block: usize, offset: i64) -> Option<usize> {
        let Some(index) = item_of_block(&self.layout.flow.items, block) else {
            return self.cells.get(&block).copied();
        };
        let FlowItem::Para(p) = self.layout.flow.items.get(index)? else {
            return None;
        };
        let found = self.lines.get(&index)?;
        let m = &p.measured.measured;
        let mut line_of: HashMap<i64, usize> = HashMap::new();
        for (number, line) in m.lines.iter().enumerate() {
            for piece in m.pieces.get(line.start..line.end).unwrap_or_default() {
                if piece.source >= 0 && !matches!(piece.kind, PieceKind::Tab | PieceKind::Break) {
                    line_of.entry(piece.source).or_insert(number);
                }
            }
        }
        let line = line_of
            .get(&offset)
            .or_else(|| line_of.get(&(offset - 1)))
            .copied()
            .unwrap_or(0);
        found.get(line.min(found.len().saturating_sub(1))).copied()
    }

    /// `NUMPAGES`: every page, blank ones included; unknown past a stop.
    fn page_count(&self) -> Option<i64> {
        let p = &self.layout.pagination;
        if p.stopped.is_some() {
            return None;
        }
        Some(i64::try_from(p.info.len().max(1)).unwrap_or(i64::MAX))
    }

    /// `SECTIONPAGES`: the pages a section has content on.
    fn section_pages(&self, section: usize) -> Option<i64> {
        let p = &self.layout.pagination;
        let items = &self.layout.flow.items;
        if let Some(stopped) = &p.stopped {
            if items.get(stopped.item).is_some_and(|i| section >= i.section()) {
                return None;
            }
        }
        let mut pages = BTreeSet::new();
        for page in &p.pages {
            let lines = page.lines.iter().map(|l| l.item);
            let rows = page.rows.iter().map(|r| r.item);
            if lines
                .chain(rows)
                .any(|item| items.get(item).is_some_and(|i| i.section() == section))
            {
                pages.insert(page.page);
            }
        }
        i64::try_from(pages.len()).ok()
    }
}

/// Body paragraph (document order, cells included) -> the pages (0-based, blank pages
/// counted) its drawn lines are on (`paginate.drawn_pages`): a paragraph's lines that draw
/// nothing left out; a table cell's paragraphs on the page their part of the row is on.
pub(crate) fn drawn_pages(layout: &DocumentLayout) -> HashMap<usize, BTreeSet<usize>> {
    let items = &layout.flow.items;
    let mut out: HashMap<usize, BTreeSet<usize>> = HashMap::new();
    for page in &layout.pagination.pages {
        for line in &page.lines {
            let Some(FlowItem::Para(p)) = items.get(line.item) else {
                continue;
            };
            let undrawn = match page.broken.get(&line.item) {
                Some(again) => again.measured.undrawn.contains(&line.line),
                None => p.measured.undrawn.contains(&line.line),
            };
            if !undrawn {
                out.entry(p.block).or_default().insert(page.page);
            }
        }
    }
    for_cell_lines(layout, &mut |block, page| {
        out.entry(block).or_default().insert(page);
    });
    out
}

/// The pages (0-based, blank pages counted) each `STYLEREF` occurrence is drawn on
/// (`_occurrence_pages`): a paragraph's, every page a line of it is on; a character
/// style's run, its first character's page.
pub(crate) fn occurrence_pages(
    layout: &DocumentLayout,
    drawn: &HashMap<usize, BTreeSet<usize>>,
    occurrences: &[styleref::Occurrence],
) -> Vec<BTreeSet<usize>> {
    let mut pages: Option<Pages<'_>> = None;
    occurrences
        .iter()
        .map(|o| {
            if o.offset == 0 {
                return drawn.get(&o.block).cloned().unwrap_or_default();
            }
            let pages = pages.get_or_insert_with(|| Pages::new(layout, None));
            let offset = i64::try_from(o.offset).unwrap_or(i64::MAX);
            pages.page_of(o.block, offset).into_iter().collect()
        })
        .collect()
}

/// Bookmark name -> `(paragraph, run)` of its start, the first of a name winning.
fn bookmarks(paragraphs: &[Arc<Paragraph>]) -> HashMap<String, (usize, usize)> {
    let mut out = HashMap::new();
    for (block, paragraph) in paragraphs.iter().enumerate() {
        for mark in paragraph.bookmarks.iter() {
            if let (true, Some(name)) = (mark.start, mark.name.as_deref().filter(|n| !n.is_empty())) {
                out.entry(name.to_owned()).or_insert((block, mark.run));
            }
        }
    }
    out
}

fn offset_of(paragraph: &Paragraph, run: usize) -> i64 {
    let count: usize = paragraph.runs.iter().take(run).map(|r| r.text.chars().count()).sum();
    i64::try_from(count).unwrap_or(i64::MAX)
}

/// Each page-number field's text on the page its line is on, from a layout of
/// `document`; a field that cannot be computed keeps Word's cached result.
pub fn field_values(document: &WordDocument, layout: &DocumentLayout, keys: &[FieldKey]) -> FieldValues {
    let mut out = FieldValues::new();
    if keys.is_empty() {
        return out;
    }
    let (paragraphs, _) = body_paragraphs(&document.body);
    let field_of = |(block, k): (usize, usize)| paragraphs.get(block)?.runs.get(k)?.field.clone();
    let refers = keys
        .iter()
        .any(|key| field_of(*key).is_some_and(|f| &*f.keyword == "PAGEREF"));
    let targets: HashMap<String, (usize, usize)> = if refers { bookmarks(&paragraphs) } else { HashMap::new() };
    // Only the paragraphs holding fields, and the bookmarks PAGEREF points at, are looked up.
    let mut asked: BTreeSet<usize> = keys.iter().map(|(block, _)| *block).collect();
    asked.extend(targets.values().map(|(block, _)| *block));
    let pages = Pages::new(layout, Some(&asked));
    let mut section_pages: HashMap<usize, Option<i64>> = HashMap::new();
    let format_of = |section: usize| {
        layout
            .sections
            .get(section)
            .and_then(|s| s.props.page_number_format.as_deref().map(str::to_owned))
    };
    for &(block, k) in keys {
        let Some(paragraph) = paragraphs.get(block) else {
            continue;
        };
        let Some(field) = paragraph.runs.get(k).and_then(|r| r.field.as_ref()) else {
            continue;
        };
        let text = if &*field.keyword == "PAGEREF" {
            let target = argument(&field.instruction).and_then(|name| targets.get(&name).copied());
            let info = target
                .and_then(|(b, run)| pages.page_of(b, paragraphs.get(b).map_or(0, |p| offset_of(p, run))))
                .and_then(|page| layout.pagination.info.get(page));
            info.and_then(|i| {
                field_text(
                    &field.instruction,
                    "PAGEREF",
                    i.number,
                    (None, None),
                    format_of(i.section).as_deref(),
                )
                .ok()
            })
        } else {
            let info = pages
                .page_of(block, offset_of(paragraph, k))
                .and_then(|page| layout.pagination.info.get(page));
            info.and_then(|i| {
                let in_section = *section_pages
                    .entry(i.section)
                    .or_insert_with(|| pages.section_pages(i.section));
                let counts = (pages.page_count(), in_section);
                field_text(
                    &field.instruction,
                    &field.keyword,
                    i.number,
                    counts,
                    format_of(i.section).as_deref(),
                )
                .ok()
            })
        };
        if let Some(text) = text {
            out.insert((block, k), text);
        }
    }
    out
}

/// A walk over the body's paragraphs in document order (cells included), rebuilding
/// the ones a function changes and sharing every other block.
/// What a [`Rebuild`] asks of each paragraph (its index, the paragraph): its replacement,
/// if it has one.
pub(crate) type Change<'f> = &'f mut dyn FnMut(usize, &Arc<Paragraph>) -> Option<Arc<Paragraph>>;

pub(crate) struct Rebuild<'f> {
    pub(crate) next: usize,
    pub(crate) change: Change<'f>,
    /// Whether a table holding the paragraphs of a range may change: one that cannot is
    /// shared without its cells being walked (`None`: every table is walked).
    pub(crate) touches: Option<&'f dyn Fn(std::ops::Range<usize>) -> bool>,
}

impl Rebuild<'_> {
    pub(crate) fn block(&mut self, block: &Block) -> Block {
        match block {
            Block::Paragraph(p) => {
                let index = self.next;
                self.next += 1;
                Block::Paragraph((self.change)(index, p).unwrap_or_else(|| Arc::clone(p)))
            }
            Block::Table(t) => {
                if let Some(touches) = self.touches {
                    let count = crate::tables::table_paragraphs(t);
                    if !touches(self.next..self.next + count) {
                        self.next += count;
                        return Block::Table(Arc::clone(t));
                    }
                }
                let rows: Vec<_> = t
                    .rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|cell| cell.iter().map(|b| self.block(b)).collect::<Arc<[Block]>>())
                            .collect::<Arc<[_]>>()
                    })
                    .collect();
                let same = rows.iter().zip(t.rows.iter()).all(|(a, b)| {
                    a.iter()
                        .zip(b.iter())
                        .all(|(x, y)| x.iter().zip(y.iter()).all(|(p, q)| same_block(p, q)))
                });
                if same {
                    Block::Table(Arc::clone(t))
                } else {
                    Block::Table(Arc::new(Table {
                        rows: rows.into(),
                        ..(**t).clone()
                    }))
                }
            }
        }
    }
}

fn same_block(a: &Block, b: &Block) -> bool {
    match (a, b) {
        (Block::Paragraph(x), Block::Paragraph(y)) => Arc::ptr_eq(x, y),
        (Block::Table(x), Block::Table(y)) => Arc::ptr_eq(x, y),
        _ => false,
    }
}

/// A paragraph with its fields' values in: the original, the values, the result.
type Substituted = (Arc<Paragraph>, Vec<(usize, String)>, Arc<Paragraph>);

/// Paragraphs with their fields' values in, kept between versions: the same paragraph
/// (the model shares an unchanged one's `Arc`) with the same values is the same
/// substituted paragraph, so its measure is kept by its `Arc` too. Each entry holds its
/// original, so an address names one paragraph for as long as it is kept.
#[derive(Debug, Default)]
pub struct Substitutions {
    kept: HashMap<usize, Substituted>,
    used: std::collections::HashSet<usize>,
}

impl Substitutions {
    /// Forget the paragraphs no substitution since the last call used.
    pub fn retain_used(&mut self) {
        let used = std::mem::take(&mut self.used);
        self.kept.retain(|k, _| used.contains(k));
    }

    fn get(&mut self, paragraph: &Arc<Paragraph>, values: &[(usize, String)]) -> Arc<Paragraph> {
        let key = Arc::as_ptr(paragraph) as usize;
        self.used.insert(key);
        if let Some((original, kept_values, substituted)) = self.kept.get(&key) {
            if Arc::ptr_eq(original, paragraph) && kept_values.as_slice() == values {
                return Arc::clone(substituted);
            }
        }
        let runs: Vec<Run> = paragraph
            .runs
            .iter()
            .enumerate()
            .map(|(k, run)| match values.iter().find(|(r, _)| *r == k) {
                Some((_, text)) => Run {
                    text: Arc::from(text.as_str()),
                    ..run.clone()
                },
                None => run.clone(),
            })
            .collect();
        let substituted = Arc::new(Paragraph {
            runs: runs.into(),
            ..(**paragraph).clone()
        });
        self.kept
            .insert(key, (Arc::clone(paragraph), values.to_vec(), Arc::clone(&substituted)));
        substituted
    }
}

/// The document with its body's page-number fields' runs holding `values`
/// (`substitute_document`): every other block shared with `document`, and a paragraph
/// substituted alike before taken from `kept`.
pub fn substitute(document: &WordDocument, values: &FieldValues, kept: &mut Substitutions) -> WordDocument {
    let mut change = |index: usize, paragraph: &Arc<Paragraph>| -> Option<Arc<Paragraph>> {
        let changed: Vec<(usize, String)> = values
            .range((index, 0)..(index + 1, 0))
            .filter(|((_, k), text)| {
                paragraph
                    .runs
                    .get(*k)
                    .is_some_and(|run| run.field.is_some() && **text != *run.text)
            })
            .map(|((_, k), text)| (*k, text.clone()))
            .collect();
        (!changed.is_empty()).then(|| kept.get(paragraph, &changed))
    };
    let touches = |range: std::ops::Range<usize>| values.range((range.start, 0)..(range.end, 0)).next().is_some();
    let mut walk = Rebuild {
        next: 0,
        change: &mut change,
        touches: Some(&touches),
    };
    let body: Vec<Block> = document.body.iter().map(|b| walk.block(b)).collect();
    let mut out = document.clone();
    out.body = body.into();
    out
}
