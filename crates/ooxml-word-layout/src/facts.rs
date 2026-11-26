//! Owns: the layout facts a check reads (docx-agent's `layout.DocumentLayout` over
//! docx2svg's pages, lane L3d): every page -- blank ones included -- with what the
//! paginator knows of it, its frame, the body's lines it draws by paragraph key (top,
//! pitch, baseline, column) and the body blocks it draws something of
//! ([`PageFacts::blocks`], docx2svg's `Page.blocks`); a page's header's and footer's
//! lines ([`story_line_facts`], laid out with the page's field values, not painted); where
//! a paragraph or table is drawn, page by page and column by column
//! ([`LayoutFacts::placements`], `where`); the pages a block is on ([`LayoutFacts::pages_of`]); and each top-level block's keeps as the
//! paginator read them ([`BlockKeeps`]). One page's facts are made on their own
//! ([`PageFacts::of`]), once per layout ([`DocumentLayout::page_facts`]).
//!
//! Blocks are named by key ([`Anchor`]): a page whose facts a relayout keeps keeps them
//! while its blocks' keys are the same, wherever their paths moved. Naming them -- a path
//! (docx2svg's `w:body/w:tbl[1]/w:tr[2]/w:tc[1]/w:p[1]`) or a tool's id -- is the edge's,
//! through the version's [`VersionNames`].
//!
//! Must not: lay anything out, judge what it reports (verdicts are the check's), or hold
//! anything but the facts of one layout.

use crate::engine::DocumentLayout;
use crate::flow::{body_paragraphs, FlowItem};
use crate::geometry::{DrawnLine, LineOf, LineSource, PageFrame};
use crate::notes::{separator, NotesPart};
use crate::paginate::PageInfo;
use crate::px::Px;
use ooxml_word_model::docx::VersionNames;
use ooxml_word_model::ids::{Anchor, BlockKey, StoryIx};
use ooxml_word_model::model::{Block, Paragraph};
use ooxml_word_model::value::Text;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// One line a page draws, as a check reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineFact {
    /// Where it comes from in the flow.
    pub source: LineSource,
    /// Its paragraph's key (`None`: a paragraph no story's index holds, a notes separator
    /// the layout makes).
    pub anchor: Option<Anchor>,
    /// The layout model's path of a paragraph the layout makes (a notes separator, which no
    /// index holds): what an edge names its line by; `None` for a line with a key.
    pub made: Option<Text>,
    /// Its number in the paragraph.
    pub number: usize,
    /// The exact top of its pitch, px from the page's top.
    pub top: Px,
    /// Its pitch.
    pub pitch: Px,
    /// Its baseline's device-pixel row.
    pub baseline: i64,
    /// Its text column, in a section of several columns.
    pub column: Option<usize>,
}

/// One page's facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageFacts {
    /// Its 0-based place, blank pages counted.
    pub page: usize,
    /// What the paginator knows of it.
    pub info: PageInfo,
    /// Its size and text area.
    pub frame: PageFrame,
    /// The body's lines it draws, in drawing order.
    pub lines: Vec<LineFact>,
    /// The body's blocks it draws something of (`Page.blocks`), by anchor: each paragraph,
    /// table, row and cell (a cell as its table's [`ooxml_word_model::ids::Sub::Cell`]) of
    /// its lines' paragraphs, once each, in drawing order.
    pub blocks: Vec<Anchor>,
}

/// A header's or footer's line a page draws, as a check reads it: its story's kind and
/// part, and the line (its paragraph's path, `w:hdr/w:p[1]`, beside it: the page's
/// header and footer are laid out for the page, not kept by key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryLineFact {
    /// `header` or `footer`.
    pub kind: &'static str,
    /// Its part (`word/header1.xml`).
    pub part: Text,
    /// Its paragraph's path.
    pub path: Text,
    /// The line.
    pub line: LineFact,
}

/// The header's and footer's lines of a page's parts (`WordLayout::page_parts`), in
/// drawing order: the header's, then the footer's.
pub fn story_line_facts(parts: &crate::geometry::stories::PageParts) -> Vec<StoryLineFact> {
    let mut out = Vec::new();
    for story in &parts.stories {
        let (paragraphs, _) = body_paragraphs(&story.blocks);
        for line in &story.walked.lines {
            let Some(paragraph) = line.block.and_then(|b| paragraphs.get(b)) else {
                continue;
            };
            out.push(StoryLineFact {
                kind: story.kind,
                part: story.part.clone(),
                path: paragraph.path.clone(),
                line: LineFact {
                    source: line.source,
                    anchor: paragraph.key.map(Anchor::whole),
                    made: paragraph.key.is_none().then(|| paragraph.path.clone()),
                    number: line.number,
                    top: line.top,
                    pitch: line.pitch,
                    baseline: line.baseline,
                    column: line.column,
                },
            });
        }
    }
    out
}

/// Where a block is drawn on one page in one text column (`Placement`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// The 0-based page.
    pub page: usize,
    /// The first of its lines' tops there.
    pub top: Px,
    /// The last of its lines' bottoms (top plus pitch).
    pub bottom: Px,
    /// How many of its lines are there.
    pub lines: usize,
    /// The text column (0 in a section of one column).
    pub column: usize,
}

/// A top-level block's keeps, as the paginator read them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKeeps {
    /// A paragraph's.
    Paragraph {
        /// `w:keepNext`.
        keep_next: bool,
        /// `w:keepLines`.
        keep_lines: bool,
        /// `w:pageBreakBefore`.
        page_break_before: bool,
    },
    /// A table's: per row, whether it keeps with the next.
    Table {
        /// Each row's keep with the next.
        rows: Vec<bool>,
        /// How many rows at its top repeat as headers.
        header_rows: usize,
    },
}

/// One layout's facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutFacts {
    /// Every page, blank ones included: each shared with the layout before where it kept
    /// the page ([`LayoutFacts::after`]), so a reader can tell a page unchanged by identity.
    pub pages: Vec<Arc<PageFacts>>,
    /// The top-level body block the layout stopped at, if it did.
    pub stopped: Option<usize>,
    /// Each top-level body block laid out in the flow, by its index in the body.
    pub keeps: BTreeMap<usize, BlockKeeps>,
}

/// What a drawn line can be of: the body's paragraphs (cells included) and its top-level
/// blocks, and the endnotes' continuation separator, where the layout draws one.
#[derive(Debug, Clone)]
pub(crate) struct LineKeys {
    paragraphs: Arc<[Arc<Paragraph>]>,
    opening: Option<Arc<Paragraph>>,
}

impl LineKeys {
    /// What `layout`'s lines can be of.
    pub(crate) fn of(layout: &DocumentLayout) -> LineKeys {
        let opening = separator(&layout.document, "continuationSeparator", NotesPart::Endnotes)
            .iter()
            .find_map(|b| match b {
                Block::Paragraph(p) => Some(Arc::clone(p)),
                Block::Table(_) => None,
            });
        LineKeys {
            paragraphs: Arc::clone(layout.body_paragraphs()),
            opening,
        }
    }

    /// The paragraph `line` is of, where the layout names one.
    fn paragraph<'k>(&'k self, line: LineOf, body: &'k [Block]) -> Option<&'k Paragraph> {
        match (line.source, line.block, line.body) {
            (LineSource::Opening { .. }, ..) => self.opening.as_deref(),
            (_, Some(block), _) => self.paragraphs.get(block).map(|p| &**p),
            (_, None, Some(index)) => match body.get(index) {
                Some(Block::Paragraph(p)) => Some(p),
                _ => None,
            },
            (_, None, None) => None,
        }
    }

    /// The key of the paragraph `line` is of, where an index holds it.
    fn key(&self, line: LineOf, body: &[Block]) -> Option<BlockKey> {
        self.paragraph(line, body)?.key
    }
}

fn line_fact(line: &DrawnLine, keys: &LineKeys, body: &[Block]) -> Option<LineFact> {
    let paragraph = keys.paragraph(LineOf::from(line), body)?;
    Some(LineFact {
        source: line.source,
        anchor: paragraph.key.map(Anchor::whole),
        made: paragraph.key.is_none().then(|| paragraph.path.clone()),
        number: line.number,
        top: line.top,
        pitch: line.pitch,
        baseline: line.baseline,
        column: line.column,
    })
}

impl PageFacts {
    /// Page `page`'s facts (0-based, blank pages counted); `None` past the last.
    pub fn of(layout: &DocumentLayout, page: usize) -> Option<PageFacts> {
        let geometry = layout.page_geometry(page)?;
        let keys = layout.line_keys();
        let body = &layout.document.body;
        let lines: Vec<LineFact> = geometry.lines.iter().filter_map(|l| line_fact(l, keys, body)).collect();
        let blocks = page_blocks(&layout.document.names, &lines);
        Some(PageFacts {
            page,
            info: geometry.info,
            frame: geometry.frame,
            lines,
            blocks,
        })
    }
}

/// The body blocks `lines` draw something of, by anchor, once each in drawing order: each
/// anchored body line's paragraph and the tables, rows and cells it is in, found through
/// the version's `names`.
fn page_blocks(names: &VersionNames, lines: &[LineFact]) -> Vec<Anchor> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut done = HashSet::new();
    for line in lines {
        let Some(anchor) = line.anchor.filter(|a| a.block.story() == StoryIx::BODY) else {
            continue;
        };
        if !done.insert(anchor.block) {
            continue;
        }
        let Some(chain) = names.blocks_of(anchor.block) else {
            continue;
        };
        out.extend(chain.into_iter().filter(|a| seen.insert(*a)));
    }
    out
}

impl LayoutFacts {
    /// The facts of `layout`, every page's.
    pub fn of(layout: &DocumentLayout) -> LayoutFacts {
        let pages = (0..layout.page_count())
            .filter_map(|k| PageFacts::of(layout, k).map(Arc::new))
            .collect();
        LayoutFacts {
            pages,
            stopped: stopped(layout),
            keeps: keeps(layout),
        }
    }

    /// The facts of `layout`, laid out again from `before`, whose facts are `known`: a page
    /// it kept as `before` laid it out -- the very same laid page, drawing the very same
    /// flow items of blocks with the same keys, with the same number and story, its frame
    /// made of the same section properties and header and footer heights -- takes its facts
    /// from `known`; the rest are made. Equal to [`LayoutFacts::of`] (an edit pays for the
    /// pages it moved).
    pub fn after(layout: &DocumentLayout, before: &DocumentLayout, known: &LayoutFacts) -> LayoutFacts {
        let pages: Vec<Arc<PageFacts>> = (0..layout.page_count())
            .filter_map(|k| kept_page(layout, before, known, k).or_else(|| PageFacts::of(layout, k).map(Arc::new)))
            .collect();
        LayoutFacts {
            pages,
            stopped: stopped(layout),
            keeps: keeps(layout),
        }
    }

    /// Where the paragraph `key` is drawn: one placement per page and text column, in page
    /// order and, on a page, in the order its columns are first drawn (`where`).
    pub fn placements(&self, key: BlockKey) -> Vec<Placement> {
        let mut out: Vec<Placement> = Vec::new();
        for page in &self.pages {
            let first = out.len();
            for line in page.lines.iter().filter(|l| l.anchor.map(|a| a.block) == Some(key)) {
                let column = line.column.unwrap_or(0);
                let bottom = line.top + line.pitch;
                let found = out
                    .get_mut(first..)
                    .and_then(|mine| mine.iter_mut().find(|p| p.column == column));
                match found {
                    Some(p) => {
                        p.top = p.top.min(line.top);
                        p.bottom = p.bottom.max(bottom);
                        p.lines += 1;
                    }
                    None => out.push(Placement {
                        page: page.page,
                        top: line.top,
                        bottom,
                        lines: 1,
                        column,
                    }),
                }
            }
        }
        out
    }

    /// Each paragraph's first page (0-based), by key: the first page whose lines draw it.
    /// What [`DocumentLayout::first_pages`] makes without placing a line.
    pub fn first_pages(&self) -> HashMap<BlockKey, usize> {
        let mut first = HashMap::new();
        for page in &self.pages {
            for key in page.lines.iter().filter_map(|l| l.anchor) {
                first.entry(key.block).or_insert(page.page);
            }
        }
        first
    }

    /// Every 0-based page a block (a paragraph, table, row, or a cell as its table's
    /// [`ooxml_word_model::ids::Sub::Cell`]) draws something on, in order (`pages_of`).
    pub fn pages_of(&self, block: Anchor) -> Vec<usize> {
        self.pages
            .iter()
            .filter(|p| p.blocks.contains(&block))
            .map(|p| p.page)
            .collect()
    }

    /// [`LayoutFacts::pages_of`] for each of `blocks`, in their order, in one pass over the
    /// pages.
    pub fn pages_of_each(&self, blocks: &[Anchor]) -> Vec<Vec<usize>> {
        let mut wanted: HashMap<Anchor, Vec<usize>> = HashMap::new();
        for (k, block) in blocks.iter().enumerate() {
            wanted.entry(*block).or_default().push(k);
        }
        let mut out = vec![Vec::new(); blocks.len()];
        for page in &self.pages {
            for block in &page.blocks {
                for &k in wanted.get(block).map_or(&[][..], Vec::as_slice) {
                    if let Some(pages) = out.get_mut(k) {
                        pages.push(page.page);
                    }
                }
            }
        }
        out
    }
}

/// Each body paragraph's first page (0-based) in `layout`, by key: its pages' lines as
/// [`LayoutFacts::of`] reads them -- those of a page of a known section -- found without
/// placing a line ([`crate::geometry::body_lines_of`]).
pub(crate) fn first_pages_of(layout: &DocumentLayout) -> HashMap<BlockKey, usize> {
    let mut first = HashMap::new();
    for (page, keys) in layout.page_keys().iter().enumerate() {
        for key in keys.iter() {
            first.entry(*key).or_insert(page);
        }
    }
    first
}

/// The keys of the body paragraphs page `page` of `layout` draws lines of, each once, in
/// the order first drawn (none for a blank page or one of an unknown section): the page's
/// part of [`first_pages_of`].
pub(crate) fn page_keys_of(layout: &DocumentLayout, page: usize) -> Arc<[BlockKey]> {
    let section = layout
        .pagination
        .info
        .get(page)
        .and_then(|info| layout.sections.get(info.section));
    let (Some(section), Some(laid)) = (section, layout.laid_page(page)) else {
        return Arc::from(Vec::new());
    };
    let (keys, body) = (layout.line_keys(), &layout.document.body);
    let height = crate::geometry::device_px(section.props.page_size.height);
    let mut seen = HashSet::new();
    crate::geometry::body_lines_of(layout, laid, height)
        .into_iter()
        .filter_map(|line| keys.key(line, body))
        .filter(|key| seen.insert(*key))
        .collect()
}

/// Page `page`'s keys ([`page_keys_of`]) as `before` (laid out before `layout`) made them,
/// where `layout` kept the page as `before` laid it out: the very same laid page, of the
/// same place and section, its page as tall, its blocks' keys the same
/// ([`same_page_blocks`]).
pub(crate) fn kept_page_keys(
    layout: &DocumentLayout,
    before: &DocumentLayout,
    known: &[Arc<[BlockKey]>],
    page: usize,
) -> Option<Arc<[BlockKey]>> {
    let info = layout.pagination.info.get(page)?;
    let height = |l: &DocumentLayout| l.sections.get(info.section).map(|s| s.props.page_size.height);
    let laid = laid_arc(layout, page)?;
    let same = layout.mode15 == before.mode15
        && Arc::ptr_eq(laid, laid_arc(before, page)?)
        && before.pagination.info.get(page) == Some(info)
        && height(layout) == height(before)
        && same_page_blocks(layout, before, laid);
    same.then(|| known.get(page).cloned()).flatten()
}

/// Whether the blocks the flow items of `laid` (a page both layouts lay out the same)
/// draw have the same keys in both: each item's top-level body block, every paragraph in
/// it, checked by key (ADR-008: a page kept across versions is checked by its blocks'
/// keys, never by their paths or addresses).
fn same_page_blocks(layout: &DocumentLayout, before: &DocumentLayout, laid: &crate::paginate::LaidPage) -> bool {
    let (now, then) = (&layout.document.body, &before.document.body);
    let mut items: Vec<usize> = laid
        .lines
        .iter()
        .map(|l| l.item)
        .chain(laid.rows.iter().map(|r| r.item))
        .collect();
    items.dedup();
    items.into_iter().all(|item| {
        let body = match layout.flow.items.get(item) {
            Some(FlowItem::Para(p)) => {
                let cap = p.drop_cap.as_ref().map(|d| d.body);
                return [Some(p.body), cap]
                    .into_iter()
                    .flatten()
                    .all(|k| same_keys(now.get(k), then.get(k)));
            }
            Some(FlowItem::Table(t)) => t.body,
            Some(FlowItem::Stop(_)) | None => return true,
        };
        same_keys(now.get(body), then.get(body))
    })
}

/// Whether two blocks, and every block in them, have the same keys.
fn same_keys(a: Option<&Block>, b: Option<&Block>) -> bool {
    match (a, b) {
        (Some(Block::Paragraph(a)), Some(Block::Paragraph(b))) => a.key == b.key,
        (Some(Block::Table(a)), Some(Block::Table(b))) => {
            Arc::ptr_eq(a, b)
                || (a.key == b.key
                    && a.rows.len() == b.rows.len()
                    && a.rows.iter().zip(b.rows.iter()).all(|(x, y)| {
                        x.len() == y.len()
                            && x.iter().zip(y.iter()).all(|(c, d)| {
                                c.len() == d.len() && c.iter().zip(d.iter()).all(|(e, f)| same_keys(Some(e), Some(f)))
                            })
                    }))
        }
        _ => false,
    }
}

/// The top-level body block `layout` stopped at, if it did.
fn stopped(layout: &DocumentLayout) -> Option<usize> {
    layout
        .pagination
        .stopped
        .as_ref()
        .and_then(|s| layout.flow.items.get(s.item))
        .map(FlowItem::body)
}

/// Page `page`'s facts as `known` (`before`'s) holds them, where `layout` kept the page as
/// `before` laid it out, its blocks' keys the same (see [`LayoutFacts::after`]). The very
/// same facts, shared.
fn kept_page(
    layout: &DocumentLayout,
    before: &DocumentLayout,
    known: &LayoutFacts,
    page: usize,
) -> Option<Arc<PageFacts>> {
    let laid = laid_arc(layout, page)?;
    let old = laid_arc(before, page)?;
    let facts = known.pages.get(page).filter(|f| f.page == page)?;
    let info = layout.pagination.info.get(page)?;
    let same = layout.mode15 == before.mode15
        && Arc::ptr_eq(laid, old)
        && before.pagination.info.get(page) == Some(info)
        && facts.info == *info
        && !facts
            .lines
            .iter()
            .any(|l| matches!(l.source, LineSource::Opening { .. }))
        && same_frame(layout, before, page, info)
        && laid
            .lines
            .iter()
            .map(|l| l.item)
            .chain(laid.rows.iter().map(|r| r.item))
            .all(|item| same_flow_item(layout.flow.items.get(item), before.flow.items.get(item)))
        && same_page_blocks(layout, before, laid);
    same.then(|| Arc::clone(facts))
}

/// Page `page`'s laid page, shared.
fn laid_arc(layout: &DocumentLayout, page: usize) -> Option<&std::sync::Arc<crate::paginate::LaidPage>> {
    let pages = &layout.pagination.pages;
    let k = pages.partition_point(|p| p.page < page);
    pages.get(k).filter(|p| p.page == page)
}

/// Whether page `page` (`info`) has the same frame in both layouts: what it is made of --
/// its section's properties and its header's and footer's heights -- is the same.
fn same_frame(layout: &DocumentLayout, before: &DocumentLayout, page: usize, info: &PageInfo) -> bool {
    use crate::stories::StoryHeights;
    let section = info.section;
    let props = match (layout.sections.get(section), before.sections.get(section)) {
        (Some(a), Some(b)) => std::sync::Arc::ptr_eq(&a.props, &b.props) || a.props == b.props,
        _ => false,
    };
    let heights = |l: &DocumentLayout| match l.page_stories.get(&page) {
        Some(own) => *own,
        None => l.stories.heights(section, info.story),
    };
    props && heights(layout) == heights(before)
}

/// Whether two flow items are the very same item.
fn same_flow_item(a: Option<&FlowItem>, b: Option<&FlowItem>) -> bool {
    use std::sync::Arc;
    match (a, b) {
        (Some(FlowItem::Para(x)), Some(FlowItem::Para(y))) => Arc::ptr_eq(x, y),
        (Some(FlowItem::Table(x)), Some(FlowItem::Table(y))) => Arc::ptr_eq(x, y),
        (Some(FlowItem::Stop(x)), Some(FlowItem::Stop(y))) => Arc::ptr_eq(x, y),
        _ => false,
    }
}

/// Each top-level block's keeps, as the flow holds them.
fn keeps(layout: &DocumentLayout) -> BTreeMap<usize, BlockKeeps> {
    let mut out = BTreeMap::new();
    for item in layout.flow.items.iter() {
        match item {
            FlowItem::Para(p) => {
                out.insert(
                    p.body,
                    BlockKeeps::Paragraph {
                        keep_next: p.keep_next,
                        keep_lines: p.keep_lines,
                        page_break_before: p.page_break_before,
                    },
                );
            }
            FlowItem::Table(t) => {
                let rows = (0..t.flow.rows()).map(|r| t.flow.keeps_with_next(r)).collect();
                out.insert(
                    t.body,
                    BlockKeeps::Table {
                        rows,
                        header_rows: t.flow.header_rows(),
                    },
                );
            }
            FlowItem::Stop(_) => {}
        }
    }
    out
}
