//! Owns: `keep_with_next_at_foot` and `settings.keep_together` (docx-agent's
//! `keep_with_next`, `_table_end_at_foot`, `_kept_rows`, `keep_together`, round 24 W3 and
//! round 25 W1/W2/W5): a keep-with-next paragraph drawn last on its page with its next block
//! on a later page (unless that block starts a new page anyway); a table's last row that
//! keeps with the next drawn last on its page, the block after it on a later page; a table
//! split after a row that keeps with the next (the layout's rule, the one it paginates by);
//! and the tables kept together -- every row but the last keeping with the next -- with the
//! paragraphs whose keep is direct formatting.
//!
//! Must not: judge a keep the paginator did not read (a row's keep is the layout's), or
//! list a table kept together taller than a page more than once.

use super::tree::{is_w, page_break_at, row_of, W};
use super::{story_kind, Block, BlockKind, Facts};
use ooxml_word_layout::facts::{BlockKeeps, LineFact};
use ooxml_word_model::docx::keeps::{row_keeps, RowKeep, TableKeeps};
use ooxml_word_model::docx::StoryPart;
use ooxml_word_model::model::Block as ModelBlock;
use ooxml_xml::Element;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// A keep broken at a page's foot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeepAtFoot {
    /// A keep-with-next paragraph drawn last on its page.
    Paragraph {
        /// Its id.
        para: String,
        /// Its style's name.
        style: Option<String>,
        /// Its page.
        page: usize,
        /// The block after it.
        next: String,
        /// The page that block starts on.
        next_page: usize,
    },
    /// A table whose last row keeps with the next, drawn last on its page.
    TableEnd {
        /// The table's id.
        table: String,
        /// Its last row, from 0.
        row: usize,
        /// Its page.
        page: usize,
        /// The block after the table.
        next: String,
        /// The page that block starts on.
        next_page: usize,
    },
    /// A table split after a row that keeps with the next.
    Row {
        /// The table's id.
        table: String,
        /// The row, from 0.
        row: usize,
        /// Its page.
        page: usize,
        /// The next row.
        next_row: usize,
        /// The page the next row starts on.
        next_page: usize,
        /// The table is kept together and taller than a page (its first split only).
        taller_than_page: bool,
    },
}

/// A table kept together (`settings.keep_together`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptTogether {
    /// Its id.
    pub table: String,
    /// The 1-based pages it is drawn on.
    pub pages: Vec<usize>,
    /// The paragraphs deciding its rows' keeps whose keep is direct formatting.
    pub direct: Vec<String>,
    /// Its last row keeps with the next too.
    pub with_next: bool,
    /// Drawn on more than one page.
    pub taller_than_page: bool,
}

/// The tables kept together, of those on the pages asked for or drawn nowhere
/// (`_kept_tables`), and those of them drawn on more than one page.
#[derive(Debug, Clone, Default)]
pub struct KeptTables {
    tables: Vec<(String, Vec<usize>, Vec<String>, bool)>,
    taller: HashSet<String>,
}

/// Every story part's tables kept together (comments aside).
#[must_use]
pub fn kept_tables(facts: &Facts<'_>) -> KeptTables {
    let mut out = KeptTables::default();
    let docx = facts.docx();
    for story in &docx.parts {
        if story.story == "comments" {
            continue;
        }
        let tables = story.ids.tables();
        for placed in tables.placed() {
            // A table's id is named (and its pages found) only where it is kept together.
            let Some(rows) = row_keeps(facts.formatting(), story, &placed.path) else {
                continue;
            };
            if !TableKeeps::of(&rows).together {
                continue;
            }
            let Some(table) = tables.head(placed.ordinal) else {
                continue;
            };
            let pages = facts.table_pages_of(&table.id);
            if !pages.is_empty() && !facts.shown(pages.iter().copied()) {
                continue;
            }
            let direct = direct(story, &table.path, &rows);
            let with_next = rows.last().is_some_and(|r| r.keeps);
            if pages.len() > 1 {
                out.taller.insert(table.id.clone());
            }
            out.tables.push((table.id.clone(), pages, direct, with_next));
        }
    }
    out
}

/// The paragraphs deciding `rows`' keeps (the rows of the table at `place` in `story`)
/// whose direct keep differs from their styles'.
fn direct(story: &StoryPart, place: &[u32], rows: &[RowKeep]) -> Vec<String> {
    rows.iter()
        .filter(|r| r.declared.is_some_and(|d| d != r.styled))
        .filter_map(|r| {
            let lead: Vec<u32> = place.iter().chain(r.lead.as_deref()?).copied().collect();
            Some(story.ids.paragraph_at(&lead)?.id)
        })
        .collect()
}

/// `settings.keep_together`.
#[must_use]
pub fn keep_together(facts: &Facts<'_>, kept: &KeptTables) -> Vec<KeptTogether> {
    kept.tables
        .iter()
        .filter(|(_, pages, _, _)| pages.is_empty() || facts.shown(pages.iter().copied()))
        .map(|(table, pages, direct, with_next)| KeptTogether {
            table: table.clone(),
            pages: pages.clone(),
            direct: direct.clone(),
            with_next: *with_next,
            taller_than_page: kept.taller.contains(table),
        })
        .collect()
}

/// `keep_with_next_at_foot`: page by page, then the tables split after a keeping row.
#[must_use]
pub fn keep_with_next_at_foot(facts: &Facts<'_>, kept: &KeptTables) -> Vec<KeepAtFoot> {
    let mut out = Vec::new();
    let version = facts.version();
    for (index, page) in version.facts().pages.iter().enumerate() {
        let number = index + 1;
        if !facts.shown([number]) {
            continue;
        }
        let lowest = page
            .lines
            .iter()
            .filter(|l| story_kind(&l.path) == "body" && version.is_named(&l.path))
            .fold(None::<&LineFact>, |best, l| match best {
                Some(b) if b.top >= l.top => Some(b),
                _ => Some(l),
            });
        let Some(line) = lowest else { continue };
        let found = match facts.block_at(&line.path) {
            Some((k, block)) => paragraph_at_foot(facts, k, block, number),
            None => table_end_at_foot(facts, line, number),
        };
        out.extend(found);
    }
    out.extend(kept_rows(facts, kept));
    out
}

/// A top-level keep-with-next paragraph last on page `number`, its next block on a later
/// page that does not start a page anyway.
fn paragraph_at_foot(facts: &Facts<'_>, k: usize, block: &Block<'_>, number: usize) -> Option<KeepAtFoot> {
    let next = facts.blocks().get(k + 1)?;
    if block.kind != BlockKind::Paragraph || facts.pages_of(&block.path).last() != Some(&number) {
        return None;
    }
    let (next_page, _) = facts.first_line(next).filter(|(p, _)| *p > number)?;
    if !facts.paragraph(block.element).keep_with_next || new_page_after(facts, k) {
        return None;
    }
    Some(KeepAtFoot::Paragraph {
        para: block.id().to_owned(),
        style: facts.style_of(block.element),
        page: number,
        next: next.id().to_owned(),
        next_page,
    })
}

/// A top-level table whose last row keeps with the next, its last row's `line` last on
/// page `number`, the block after it on a later page (`_table_end_at_foot`).
fn table_end_at_foot(facts: &Facts<'_>, line: &LineFact, number: usize) -> Option<KeepAtFoot> {
    let (table, row) = row_of(&line.path)?;
    let (k, block) = facts.block_at(table)?;
    let next = facts.blocks().get(k + 1)?;
    if block.kind != BlockKind::Table {
        return None;
    }
    let rows = row_keeps(facts.formatting(), facts.docx().body()?, &block.place)?;
    let last = rows.last()?;
    if row + 1 != rows.len() || last.lead.is_none() || !last.keeps {
        return None;
    }
    if facts.table_pages(&block.path).last() != Some(&number) {
        return None;
    }
    let (next_page, _) = facts.first_line(next).filter(|(p, _)| *p > number)?;
    if new_page_after(facts, k) {
        return None;
    }
    Some(KeepAtFoot::TableEnd {
        table: block.id().to_owned(),
        row,
        page: number,
        next: next.id().to_owned(),
        next_page,
    })
}

/// The body's tables as drawn: each outermost table's rows' pages, tables in the order
/// first drawn.
fn drawn_rows(facts: &Facts<'_>) -> Vec<(String, BTreeMap<usize, BTreeSet<usize>>)> {
    let mut out: Vec<(String, BTreeMap<usize, BTreeSet<usize>>)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (index, page) in facts.version().facts().pages.iter().enumerate() {
        for line in page.lines.iter().filter(|l| story_kind(&l.path) == "body") {
            let Some((table, row)) = row_of(&line.path) else {
                continue;
            };
            let at = *seen.entry(table.to_owned()).or_insert_with(|| {
                out.push((table.to_owned(), BTreeMap::new()));
                out.len() - 1
            });
            if let Some((_, rows)) = out.get_mut(at) {
                rows.entry(row).or_default().insert(index + 1);
            }
        }
    }
    out
}

/// Tables split after a row that keeps with the next (`_kept_rows`).
fn kept_rows(facts: &Facts<'_>, kept: &KeptTables) -> Vec<KeepAtFoot> {
    let mut out = Vec::new();
    for (table, rows) in drawn_rows(facts) {
        if !facts.shown(rows.values().flatten().copied()) {
            continue;
        }
        let Some(block) = facts.block_at(&table).map(|(_, b)| b) else {
            continue;
        };
        let Some(keeps) = model_keeps(facts, &table) else {
            continue;
        };
        let own = block.element.child_elements().filter(|e| is_w(e, "tr")).count();
        let header_rows = header_rows(block.element);
        let taller = kept.taller.contains(block.id());
        for (&row, drawn) in &rows {
            let Some(after) = rows.get(&(row + 1)) else { continue };
            if row < header_rows || row >= own || row >= keeps.len() {
                continue;
            }
            let (Some(&page), Some(&next_page)) = (drawn.last(), after.first()) else {
                continue;
            };
            if next_page <= page || !facts.shown([page]) || !keeps.get(row).copied().unwrap_or(false) {
                continue;
            }
            out.push(KeepAtFoot::Row {
                table: block.id().to_owned(),
                row,
                page,
                next_row: row + 1,
                next_page,
                taller_than_page: taller,
            });
            if taller {
                break;
            }
        }
    }
    out
}

/// The rows' keeps the paginator read for the top-level table at `path`.
fn model_keeps(facts: &Facts<'_>, path: &str) -> Option<Vec<bool>> {
    let version = facts.version();
    let index = version
        .layout()
        .document
        .body
        .iter()
        .position(|b| matches!(b, ModelBlock::Table(t) if &*t.path == path))?;
    match version.facts().keeps.get(&index)? {
        BlockKeeps::Table { rows, .. } => Some(rows.clone()),
        BlockKeeps::Paragraph { .. } => None,
    }
}

/// The rows at a table's top marked to repeat as headers (`_header_rows`).
pub(crate) fn header_rows(table: &Element) -> usize {
    table
        .child_elements()
        .filter(|e| is_w(e, "tr"))
        .take_while(|row| {
            let marked = row.find(Some(W), "trPr").and_then(|p| p.find(Some(W), "tblHeader"));
            marked.is_some_and(|m| {
                let value = m.attribute(Some(W), "val").unwrap_or("1").to_lowercase();
                !matches!(value.as_str(), "0" | "false" | "off")
            })
        })
        .count()
}

/// Whether the top-level block after block `k` starts a new page whatever keeps it: a page
/// break before it (style or direct), a page break at its start or at the end of block
/// `k`, or a section break after `k` whose next section starts on a new page
/// (`_new_page_after`).
pub(crate) fn new_page_after(facts: &Facts<'_>, k: usize) -> bool {
    let (Some(block), Some(next)) = (facts.blocks().get(k), facts.blocks().get(k + 1)) else {
        return false;
    };
    if next.kind == BlockKind::Paragraph
        && (facts.paragraph(next.element).page_break_before || page_break_at(next.element, true))
    {
        return true;
    }
    if page_break_at(block.element, false) {
        return true;
    }
    let Some(own) = section_place(block) else {
        return false;
    };
    let Some(story) = facts.docx().body() else {
        return false;
    };
    let Some((body_at, body)) = story
        .root()
        .children()
        .iter()
        .enumerate()
        .find_map(|(k, n)| n.as_element().filter(|e| is_w(e, "body")).map(|e| (k, e)))
    else {
        return false;
    };
    let mut sections = Vec::new();
    let mut place = vec![u32::try_from(body_at).unwrap_or(u32::MAX)];
    section_places(body, &mut place, &mut sections);
    let at = sections.iter().position(|(p, _)| *p == own);
    let section = at.and_then(|a| sections.get(a + 1)).map(|(_, s)| *s);
    let start = section
        .and_then(|s| s.find(Some(W), "type"))
        .map_or(Some("nextPage"), |t| t.attribute(Some(W), "val"));
    matches!(start, Some("nextPage" | "oddPage" | "evenPage"))
}

/// The place of a paragraph's own section break (`w:pPr/w:sectPr`), where it has one.
fn section_place(block: &Block<'_>) -> Option<Vec<u32>> {
    let (p, properties) = child(block.element, "pPr")?;
    let (s, _) = child(properties, "sectPr")?;
    Some(block.place.iter().copied().chain([p, s]).collect())
}

/// The first `w:` child `local` of `element`, with its child index.
fn child<'e>(element: &'e Element, local: &str) -> Option<(u32, &'e Arc<Element>)> {
    element.children().iter().enumerate().find_map(|(k, n)| {
        let e = n.as_element().filter(|e| is_w(e, local))?;
        Some((u32::try_from(k).ok()?, e))
    })
}

/// Every `w:sectPr` below `element` (at `place`), in document order, with its place: found
/// by place, not by node (equal subtrees may share one).
fn section_places<'e>(element: &'e Arc<Element>, place: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, &'e Arc<Element>)>) {
    for (k, node) in element.children().iter().enumerate() {
        let Some(child) = node.as_element() else { continue };
        place.push(u32::try_from(k).unwrap_or(u32::MAX));
        if is_w(child, "sectPr") {
            out.push((place.clone(), child));
        }
        section_places(child, place, out);
        place.pop();
    }
}
