//! Owns: the body's page-number fields settled against the layout (docx2svg's
//! `_Placer._body_field_values`, `_page_of`, `_bookmark_pages`, `_page_count`,
//! `_section_pages`, `substitute_document`): which runs hold a `PAGE`, `NUMPAGES`,
//! `SECTIONPAGES` or `PAGEREF` ([`page_fields`]), what each draws on the page its line is
//! on ([`field_values`]), and the document with those values in its runs
//! ([`substitute`]). The engine lays the body out again with the values found until they
//! are the ones it was laid out with.
//!
//! Must not: compute `REF`, `SEQ` or `STYLEREF` (they do not depend on the pages: see the
//! hand-back of lane L3a), or draw a field.

mod format;

pub use format::{argument, field_text, format_value, number_text, switches, Uncomputable};

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

/// Where the body's paragraphs' lines are: per flow item, the page of each line.
struct Pages<'a> {
    layout: &'a DocumentLayout,
    item_of_block: HashMap<usize, usize>,
    lines: HashMap<usize, Vec<usize>>,
}

impl<'a> Pages<'a> {
    fn new(layout: &'a DocumentLayout) -> Pages<'a> {
        let mut item_of_block = HashMap::new();
        for (k, item) in layout.flow.items.iter().enumerate() {
            if let FlowItem::Para(p) = item {
                item_of_block.insert(p.block, k);
            }
        }
        let mut lines: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
        for page in &layout.pagination.pages {
            for line in &page.lines {
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
        Pages {
            layout,
            item_of_block,
            lines,
        }
    }

    /// The page a body paragraph's character `offset` is drawn on (`_page_of`).
    fn page_of(&self, block: usize, offset: i64) -> Option<usize> {
        let index = *self.item_of_block.get(&block)?;
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
    let pages = Pages::new(layout);
    let (paragraphs, _) = body_paragraphs(&document.body);
    let mut targets: Option<HashMap<String, (usize, usize)>> = None;
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
            let targets = targets.get_or_insert_with(|| bookmarks(&paragraphs));
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
                let counts = (pages.page_count(), pages.section_pages(i.section));
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
struct Rebuild<'f> {
    next: usize,
    change: &'f dyn Fn(usize, &Paragraph) -> Option<Paragraph>,
}

impl Rebuild<'_> {
    fn block(&mut self, block: &Block) -> Block {
        match block {
            Block::Paragraph(p) => {
                let index = self.next;
                self.next += 1;
                match (self.change)(index, p) {
                    Some(changed) => Block::Paragraph(Arc::new(changed)),
                    None => Block::Paragraph(Arc::clone(p)),
                }
            }
            Block::Table(t) => {
                let before = self.next;
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
                if same || before == self.next {
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

/// The document with its body's page-number fields' runs holding `values`
/// (`substitute_document`): every other block shared with `document`.
pub fn substitute(document: &WordDocument, values: &FieldValues) -> WordDocument {
    let change = |index: usize, paragraph: &Paragraph| -> Option<Paragraph> {
        let mut changed = false;
        let runs: Vec<Run> = paragraph
            .runs
            .iter()
            .enumerate()
            .map(|(k, run)| match values.get(&(index, k)) {
                Some(text) if run.field.is_some() && **text != *run.text => {
                    changed = true;
                    Run {
                        text: Arc::from(text.as_str()),
                        ..run.clone()
                    }
                }
                _ => run.clone(),
            })
            .collect();
        changed.then(|| Paragraph {
            runs: runs.into(),
            ..paragraph.clone()
        })
    };
    let mut walk = Rebuild {
        next: 0,
        change: &change,
    };
    let body: Vec<Block> = document.body.iter().map(|b| walk.block(b)).collect();
    let mut out = document.clone();
    out.body = body.into();
    out
}
