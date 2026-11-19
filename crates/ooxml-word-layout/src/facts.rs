//! Owns: the layout facts a check reads (docx-agent's `layout.DocumentLayout` over
//! docx2svg's pages, lane L3d): every page -- blank ones included -- with what the
//! paginator knows of it, its frame, the body's lines it draws by paragraph path (top,
//! pitch, baseline, column) and the body blocks it draws something of
//! ([`PageFacts::blocks`], docx2svg's `Page.blocks`); a page's header's and footer's
//! lines ([`story_line_facts`], laid out with the page's field values, not painted); where
//! a paragraph or table is drawn, page by page and column by column
//! ([`LayoutFacts::placements`], `where`); the pages a block is on ([`LayoutFacts::pages_of`]); and each top-level block's keeps as the
//! paginator read them ([`BlockKeeps`]). One page's facts are made on their own
//! ([`PageFacts::of`]), once per layout ([`DocumentLayout::page_facts`]).
//!
//! Paths are docx2svg's: a paragraph's `w:body/w:p[3]`, a cell's
//! `w:body/w:tbl[1]/w:tr[2]/w:tc[1]/w:p[1]`. Mapping them to a tool's ids is the caller's.
//!
//! Must not: lay anything out, judge what it reports (verdicts are the check's), or hold
//! anything but the facts of one layout.

use crate::engine::DocumentLayout;
use crate::flow::{body_paragraphs, FlowItem};
use crate::geometry::{DrawnLine, LineSource, PageFrame};
use crate::notes::{separator, NotesPart};
use crate::paginate::PageInfo;
use crate::px::Px;
use ooxml_word_model::model::Block;
use ooxml_word_model::value::Text;
use std::collections::BTreeMap;

/// One line a page draws, as a check reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineFact {
    /// Where it comes from in the flow.
    pub source: LineSource,
    /// Its paragraph's path.
    pub path: Text,
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
    /// The keys of the body's blocks it draws something of (`Page.blocks`): each
    /// paragraph, table, row and cell, once each, in drawing order.
    pub blocks: Vec<Text>,
}

/// A header's or footer's line a page draws, as a check reads it: its story's kind and
/// part, and the line (its path the story's paragraph's, `w:hdr/w:p[1]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryLineFact {
    /// `header` or `footer`.
    pub kind: &'static str,
    /// Its part (`word/header1.xml`).
    pub part: Text,
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
            let Some(path) = line.block.and_then(|b| paragraphs.get(b)).map(|p| p.path.clone()) else {
                continue;
            };
            out.push(StoryLineFact {
                kind: story.kind,
                part: story.part.clone(),
                line: LineFact {
                    source: line.source,
                    path,
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
    /// Every page, blank ones included.
    pub pages: Vec<PageFacts>,
    /// The top-level body block the layout stopped at, if it did.
    pub stopped: Option<usize>,
    /// Each top-level body block laid out in the flow, by its index in the body.
    pub keeps: BTreeMap<usize, BlockKeeps>,
}

/// The body's blocks a body path is in, outermost first (`paths.block_keys`): every prefix
/// ending at a paragraph, table, row or cell step, up to the first paragraph.
pub fn block_keys(path: &str) -> Vec<String> {
    let mut steps = path.split('/');
    if steps.next() != Some("w:body") {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut prefix = String::from("w:body");
    for step in steps {
        prefix.push('/');
        prefix.push_str(step);
        let name = step.split('[').next().unwrap_or("");
        let local = name.rsplit(':').next().unwrap_or(name);
        match local {
            "p" => {
                out.push(prefix.clone());
                break;
            }
            "tbl" | "tr" | "tc" => out.push(prefix.clone()),
            "sdt" | "sdtContent" | "customXml" => {}
            _ => break,
        }
    }
    out
}

/// The paths of what a drawn line can be of: the body's paragraphs (cells included), its
/// top-level blocks, and the endnotes' continuation separator (made once per layout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinePaths {
    paragraphs: Vec<Text>,
    opening: Option<Text>,
}

impl LinePaths {
    /// The paths of `layout`'s lines.
    pub(crate) fn of(layout: &DocumentLayout) -> LinePaths {
        let (paragraphs, _) = body_paragraphs(&layout.document.body);
        let opening = separator(&layout.document, "continuationSeparator", NotesPart::Endnotes)
            .iter()
            .find_map(|b| match b {
                Block::Paragraph(p) => Some(p.path.clone()),
                Block::Table(_) => None,
            });
        LinePaths {
            paragraphs: paragraphs.iter().map(|p| p.path.clone()).collect(),
            opening,
        }
    }

    fn path(&self, line: &DrawnLine, body: &[Block]) -> Option<Text> {
        match (line.source, line.block, line.body) {
            (LineSource::Opening { .. }, ..) => self.opening.clone(),
            (_, Some(block), _) => self.paragraphs.get(block).cloned(),
            (_, None, Some(index)) => match body.get(index) {
                Some(Block::Paragraph(p)) => Some(p.path.clone()),
                _ => None,
            },
            (_, None, None) => None,
        }
    }
}

fn line_fact(line: &DrawnLine, paths: &LinePaths, body: &[Block]) -> Option<LineFact> {
    let path = paths.path(line, body)?;
    Some(LineFact {
        source: line.source,
        path,
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
        let paths = layout.line_paths();
        let body = &layout.document.body;
        let lines: Vec<LineFact> = geometry
            .lines
            .iter()
            .filter_map(|l| line_fact(l, paths, body))
            .collect();
        let mut blocks: Vec<Text> = Vec::new();
        for line in &lines {
            for key in block_keys(&line.path) {
                if !blocks.iter().any(|b| **b == *key) {
                    blocks.push(Text::from(key.as_str()));
                }
            }
        }
        Some(PageFacts {
            page,
            info: geometry.info,
            frame: geometry.frame,
            lines,
            blocks,
        })
    }
}

impl LayoutFacts {
    /// The facts of `layout`, every page's.
    pub fn of(layout: &DocumentLayout) -> LayoutFacts {
        let pages = (0..layout.page_count())
            .filter_map(|k| PageFacts::of(layout, k))
            .collect();
        LayoutFacts {
            pages,
            stopped: stopped(layout),
            keeps: keeps(layout),
        }
    }

    /// The facts of `layout`, laid out again from `before`, whose facts are `known`: a page
    /// it kept as `before` laid it out -- the very same laid page, drawing the very same
    /// flow items, with the same number, story and frame, its section's margins the same,
    /// the body's paragraphs at the same paths -- takes its facts from `known`; the rest
    /// are made. Equal to [`LayoutFacts::of`] (an edit pays for the pages it moved).
    pub fn after(layout: &DocumentLayout, before: &DocumentLayout, known: &LayoutFacts) -> LayoutFacts {
        let same_paths = layout.mode15 == before.mode15 && layout.line_paths() == before.line_paths();
        let pages = (0..layout.page_count())
            .filter_map(|k| {
                let kept = if same_paths {
                    kept_page(layout, before, known, k)
                } else {
                    None
                };
                kept.or_else(|| PageFacts::of(layout, k))
            })
            .collect();
        LayoutFacts {
            pages,
            stopped: stopped(layout),
            keeps: keeps(layout),
        }
    }

    /// Where the paragraph at `path` is drawn: one placement per page and text column, in
    /// page order and, on a page, in the order its columns are first drawn (`where`).
    pub fn placements(&self, path: &str) -> Vec<Placement> {
        let mut out: Vec<Placement> = Vec::new();
        for page in &self.pages {
            let first = out.len();
            for line in page.lines.iter().filter(|l| &*l.path == path) {
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

    /// The 0-based page a paragraph's first line is drawn on.
    pub fn first_page(&self, path: &str) -> Option<usize> {
        self.pages
            .iter()
            .find(|p| p.lines.iter().any(|l| &*l.path == path))
            .map(|p| p.page)
    }

    /// Every 0-based page a block (its key: a paragraph's, table's, row's or cell's path)
    /// draws something on, in order (`pages_of`).
    pub fn pages_of(&self, key: &str) -> Vec<usize> {
        self.pages
            .iter()
            .filter(|p| p.blocks.iter().any(|b| &**b == key))
            .map(|p| p.page)
            .collect()
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
/// `before` laid it out (see [`LayoutFacts::after`]); the paths of the body's paragraphs
/// are the caller's to have compared.
fn kept_page(layout: &DocumentLayout, before: &DocumentLayout, known: &LayoutFacts, page: usize) -> Option<PageFacts> {
    let laid = laid_arc(layout, page)?;
    let old = laid_arc(before, page)?;
    let facts = known.pages.get(page).filter(|f| f.page == page)?;
    let info = layout.pagination.info.get(page)?;
    let same = std::sync::Arc::ptr_eq(laid, old)
        && before.pagination.info.get(page) == Some(info)
        && facts.info == *info
        && !facts
            .lines
            .iter()
            .any(|l| matches!(l.source, LineSource::Opening { .. }))
        && same_margins(layout, before, info.section)
        && crate::geometry::page_frame(layout, page, info.section, info) == Some(facts.frame)
        && laid
            .lines
            .iter()
            .map(|l| l.item)
            .chain(laid.rows.iter().map(|r| r.item))
            .all(|item| same_flow_item(layout.flow.items.get(item), before.flow.items.get(item)));
    same.then(|| facts.clone())
}

/// Page `page`'s laid page, shared.
fn laid_arc(layout: &DocumentLayout, page: usize) -> Option<&std::sync::Arc<crate::paginate::LaidPage>> {
    let pages = &layout.pagination.pages;
    let k = pages.partition_point(|p| p.page < page);
    pages.get(k).filter(|p| p.page == page)
}

/// Whether section `section` has the same margins in both layouts (its columns' edges).
fn same_margins(layout: &DocumentLayout, before: &DocumentLayout, section: usize) -> bool {
    match (layout.sections.get(section), before.sections.get(section)) {
        (Some(a), Some(b)) => a.props.margins == b.props.margins,
        _ => false,
    }
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

#[cfg(test)]
mod tests {
    use super::block_keys;

    #[test]
    fn a_cell_line_is_in_its_table_row_and_cell() {
        assert_eq!(
            block_keys("w:body/w:tbl[2]/w:tr[3]/w:tc[1]/w:p[1]"),
            vec![
                "w:body/w:tbl[2]",
                "w:body/w:tbl[2]/w:tr[3]",
                "w:body/w:tbl[2]/w:tr[3]/w:tc[1]",
                "w:body/w:tbl[2]/w:tr[3]/w:tc[1]/w:p[1]",
            ]
        );
        assert_eq!(block_keys("w:body/w:sdt[1]/w:sdtContent/w:p[2]/w:r[1]").len(), 1);
        assert!(block_keys("w:hdr/w:p[1]").is_empty());
    }
}
