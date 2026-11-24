//! Owns: `column_fit` (docx-agent's `column_fit` and `_fit_columns`, round 31, R6): per
//! body table, a column whose widest line uses at most [`ROOMY_SHARE`] of its text width
//! (every paragraph of it on one line) beside a column with a paragraph drawn on at least
//! [`WRAP_LINES`] lines, and the grid widths that let both fit -- the roomy one its widest
//! line or word plus its cell margins and [`FIT_SLACK`], the room it frees (at least
//! [`MIN_FREED`]) to the wrapping one. Cells over more than one column are not counted.
//!
//! Must not: word an entry (the tool layer does) or cap a list.

use super::tree::{self, cell_line, is_w, W};
use super::{points, round, Facts};
use crate::page_read::PageRead;
use ooxml_xml::Element;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// A column is roomy when its widest line uses at most this share of its text width...
pub const ROOMY_SHARE: f64 = 0.6;
/// ... and it would give its neighbour at least this, pt;
pub const MIN_FREED: f64 = 18.0;
/// a column wraps heavily when one of its cells has a paragraph drawn on this many lines.
pub const WRAP_LINES: usize = 3;
/// Room left beside the roomy column's widest line, pt.
pub const FIT_SLACK: f64 = 3.0;

/// One column of a `column_fit` entry, pt.
#[derive(Debug, Clone, PartialEq)]
pub struct FitColumn {
    /// Its grid column, from 0.
    pub column: usize,
    /// The grid column's width.
    pub width: f64,
    /// Its widest line as drawn.
    pub widest_line: f64,
    /// Its widest run without a space.
    pub longest_word: f64,
    /// The most lines one of its paragraphs is drawn on.
    pub lines: usize,
}

/// A roomy column beside one that wraps.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnFit {
    /// The table's id.
    pub table: String,
    /// The roomy column.
    pub roomy: FitColumn,
    /// The one that wraps.
    pub wraps: FitColumn,
    /// Every grid column's suggested width, pt.
    pub widths: Vec<f64>,
    /// The table's 1-based pages.
    pub pages: Vec<usize>,
}

/// One page's contribution to `column_fit` (the page tier's, [`crate::page_entry`]): the
/// body cells it draws lines of, table by table in the order first drawn, with what each
/// cell's lines measure. A pure function of the page's read: which tables are asked for,
/// their elements and their ids are the fold's ([`column_fit`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CellsPart {
    tables: Vec<PageTable>,
}

/// A table's cells drawn on one page.
#[derive(Debug, Clone, PartialEq)]
struct PageTable {
    /// The table's layout path.
    path: String,
    cells: Vec<PageCell>,
}

/// One cell's lines drawn on one page.
#[derive(Debug, Clone, PartialEq)]
struct PageCell {
    /// Its row and its place in the row, from 0.
    at: (usize, usize),
    /// Each paragraph's lines measured on the page, in the order first drawn: its path in
    /// the cell, how many, and the widest one's ink, pt.
    paragraphs: Vec<(String, usize, f64)>,
    /// The widest run without a space of those lines, pt (0 where none is measured).
    word: f64,
}

impl CellsPart {
    /// The contribution of a page read as `read`.
    #[must_use]
    pub fn of(read: &PageRead) -> CellsPart {
        let mut tables: Vec<PageTable> = Vec::new();
        for line in read.texts.iter().filter(|l| l.story.is_none()) {
            let Some((table, row, cell, paragraph)) = cell_line(&line.path) else {
                continue;
            };
            let k = match tables.iter().position(|t| t.path == table) {
                Some(k) => k,
                None => {
                    tables.push(PageTable {
                        path: table.to_owned(),
                        cells: Vec::new(),
                    });
                    tables.len() - 1
                }
            };
            let Some(cells) = tables.get_mut(k).map(|t| &mut t.cells) else {
                continue;
            };
            let c = match cells.iter().position(|c| c.at == (row, cell)) {
                Some(c) => c,
                None => {
                    cells.push(PageCell {
                        at: (row, cell),
                        paragraphs: Vec::new(),
                        word: 0.0,
                    });
                    cells.len() - 1
                }
            };
            let (Some(seen), (Some((low, high)), Some(widest))) = (cells.get_mut(c), (line.ink, line.widest)) else {
                continue;
            };
            let width = points(high - low);
            match seen.paragraphs.iter_mut().find(|(p, ..)| p == paragraph) {
                Some((_, lines, wide)) => {
                    *lines += 1;
                    *wide = wide.max(width);
                }
                None => seen.paragraphs.push((paragraph.to_owned(), 1, width)),
            }
            seen.word = seen.word.max(points(widest));
        }
        CellsPart { tables }
    }

    /// What the contribution of a page read as `read` holds at most, bytes.
    #[must_use]
    pub fn weight_of(read: &PageRead) -> usize {
        let each = std::mem::size_of::<PageTable>()
            + std::mem::size_of::<PageCell>()
            + std::mem::size_of::<(String, usize, f64)>();
        let lines = read
            .texts
            .iter()
            .filter(|l| l.story.is_none() && l.path.contains("/w:tc["))
            .map(|l| each + l.path.len());
        std::mem::size_of::<CellsPart>() + lines.sum::<usize>()
    }
}

/// What one cell draws over the pages folded so far: each paragraph's lines and the widest
/// one's ink, and the widest run without a space, pt.
#[derive(Debug, Default)]
struct DrawnCell<'p> {
    lines: HashMap<&'p str, (usize, f64)>,
    word: f64,
}

/// A table's cells as drawn, by `(row, cell)` from 0, and the pages drawing them.
#[derive(Debug, Default)]
struct DrawnTable<'p> {
    pages: BTreeSet<usize>,
    cells: BTreeMap<(usize, usize), DrawnCell<'p>>,
}

/// `column_fit` of the body's tables, the most wrapped first; on the pages asked for, every
/// page of each table measured.
#[must_use]
pub fn column_fit(facts: &Facts<'_>) -> Vec<ColumnFit> {
    let (order, drawn) = drawn_cells(facts);
    let Some(body) = facts.docx().body() else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for path in order {
        let Some(table) = drawn.get(path) else { continue };
        let pages: Vec<usize> = table.pages.iter().copied().collect();
        if !facts.shown(pages.iter().copied()) {
            continue;
        }
        if !table.wraps() {
            continue; // no column can wrap: its element and margins are not read
        }
        let Some((element, _)) = tree::resolve(body, path) else {
            continue;
        };
        let fits = fit_columns(facts, element, &table.cells, path);
        if fits.is_empty() {
            continue;
        }
        let id = facts.version().id_of(path).unwrap_or(path).to_owned();
        for (roomy, wraps, widths) in fits {
            entries.push(ColumnFit {
                table: id.clone(),
                roomy,
                wraps,
                widths,
                pages: pages.clone(),
            });
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.wraps.lines));
    entries
}

impl DrawnTable<'_> {
    /// Whether one of its paragraphs is drawn on [`WRAP_LINES`] lines at least: without
    /// one, no column wraps and the table has no entry.
    fn wraps(&self) -> bool {
        self.cells
            .values()
            .any(|c| c.lines.values().any(|&(lines, _)| lines >= WRAP_LINES))
    }
}

/// Each body table's cells as drawn, with the tables in the order first drawn: a fold over
/// the pages' contributions ([`CellsPart`]); with pages asked for, only the tables drawn on
/// them, on every page each is.
fn drawn_cells<'f>(facts: &'f Facts<'_>) -> (Vec<&'f str>, HashMap<&'f str, DrawnTable<'f>>) {
    let pages = &facts.version().facts().pages;
    let body_cells = |index: usize| {
        pages
            .get(index)
            .into_iter()
            .flat_map(|p| p.lines.iter())
            .filter(|l| super::story_kind(&l.path) == "body")
            .filter_map(|l| cell_line(&l.path).map(|(t, ..)| t))
    };
    let shown: Option<BTreeSet<&str>> = (!facts.whole()).then(|| {
        (0..pages.len())
            .filter(|i| facts.shown([i + 1]))
            .flat_map(body_cells)
            .collect()
    });
    let wanted = |table: &str| shown.as_ref().is_none_or(|s| s.contains(table));
    let mut order: Vec<&str> = Vec::new();
    let mut drawn: HashMap<&str, DrawnTable<'_>> = HashMap::new();
    for index in 0..pages.len() {
        if !body_cells(index).any(wanted) {
            continue; // only the pages drawing a cell's line are painted
        }
        let Some(entry) = facts.entry(index) else { continue };
        for table in entry.cells().tables.iter().filter(|t| wanted(&t.path)) {
            if !drawn.contains_key(table.path.as_str()) {
                order.push(&table.path);
            }
            let seen = drawn.entry(&table.path).or_default();
            seen.pages.insert(index + 1);
            for cell in &table.cells {
                let found = seen.cells.entry(cell.at).or_default();
                for (paragraph, lines, wide) in &cell.paragraphs {
                    let (count, widest) = found.lines.entry(paragraph).or_insert((0, 0.0));
                    *count += lines;
                    *widest = widest.max(*wide);
                }
                found.word = found.word.max(cell.word);
            }
        }
    }
    (order, drawn)
}

/// A column's measures so far.
struct Measured {
    text_width: f64,
    margins: f64,
    widest_line: f64,
    longest_word: f64,
    lines: usize,
}

/// `column_fit`'s columns for one table (`_fit_columns`); none for a table of fewer than
/// two grid columns.
fn fit_columns(
    facts: &Facts<'_>,
    table: &Arc<Element>,
    cells: &BTreeMap<(usize, usize), DrawnCell<'_>>,
    path: &str,
) -> Vec<(FitColumn, FitColumn, Vec<f64>)> {
    #[allow(clippy::cast_precision_loss)]
    let grid: Vec<f64> = grid_widths(table).iter().map(|w| *w as f64 / 20.0).collect();
    if grid.len() < 2 {
        return Vec::new();
    }
    let margins = cell_margins(facts, path);
    let mut columns: BTreeMap<usize, Measured> = BTreeMap::new();
    for (r, row) in table.child_elements().filter(|e| is_w(e, "tr")).enumerate() {
        for (k, (start, span)) in row_cells(row).into_iter().enumerate() {
            let Some(seen) = cells.get(&(r, k)) else { continue };
            let Some(&width) = grid.get(start).filter(|_| span == 1) else {
                continue;
            };
            let (left, right) = margins.get(&(r, k)).copied().unwrap_or((5.4, 5.4));
            let column = columns.entry(start).or_insert(Measured {
                text_width: width - left - right,
                margins: left + right,
                widest_line: 0.0,
                longest_word: 0.0,
                lines: 0,
            });
            column.text_width = column.text_width.min(width - left - right);
            column.margins = column.margins.max(left + right);
            for &(lines, widest) in seen.lines.values() {
                column.widest_line = column.widest_line.max(widest);
                column.lines = column.lines.max(lines);
            }
            column.longest_word = column.longest_word.max(seen.word);
        }
    }
    let mut out = Vec::new();
    for (&c, roomy) in &columns {
        if let Some(found) = fit_beside(&grid, &columns, c, roomy) {
            out.push(found);
        }
    }
    out
}

/// The roomy column `c` (`roomy`) fitted beside its most wrapped neighbour, where it is
/// roomy and frees enough: the two columns and every grid column's suggested width.
fn fit_beside(
    grid: &[f64],
    columns: &BTreeMap<usize, Measured>,
    c: usize,
    roomy: &Measured,
) -> Option<(FitColumn, FitColumn, Vec<f64>)> {
    if roomy.lines != 1 || roomy.text_width <= 0.0 || roomy.widest_line > ROOMY_SHARE * roomy.text_width {
        return None;
    }
    let need = round(roomy.widest_line.max(roomy.longest_word) + FIT_SLACK + roomy.margins, 1);
    let freed = round(grid.get(c)? - need, 1);
    if freed < MIN_FREED {
        return None;
    }
    let beside = [c.checked_sub(1), Some(c + 1)]
        .into_iter()
        .flatten()
        .filter_map(|n| Some((n, columns.get(&n)?)))
        .filter(|(_, m)| m.lines >= WRAP_LINES);
    // the first of the most wrapped (Python's max)
    let (n, wraps) = beside.fold(None, |best: Option<(usize, &Measured)>, (n, m)| match best {
        Some(b) if b.1.lines >= m.lines => Some(b),
        _ => Some((n, m)),
    })?;
    let mut widths: Vec<f64> = grid.iter().map(|w| round(*w, 1)).collect();
    *widths.get_mut(c)? = need;
    *widths.get_mut(n)? = round(grid.get(n)? + freed, 1);
    let said = |column: usize, m: &Measured| FitColumn {
        column,
        width: grid.get(column).map_or(0.0, |w| round(*w, 1)),
        widest_line: m.widest_line,
        longest_word: m.longest_word,
        lines: m.lines,
    };
    Some((said(c, roomy), said(n, wraps), widths))
}

/// `w:tblGrid`'s widths, twips (`grid_widths`: an unreadable width is 0).
fn grid_widths(table: &Element) -> Vec<i64> {
    table
        .find(Some(W), "tblGrid")
        .map(|grid| {
            grid.child_elements()
                .filter(|e| is_w(e, "gridCol"))
                .map(|col| int(col.attribute(Some(W), "w")).unwrap_or(0))
                .collect()
        })
        .unwrap_or_default()
}

/// A row's own cells (row-level content controls seen through): each one's first grid
/// column (`w:gridBefore` counted) and span (`row_cells`).
fn row_cells(row: &Element) -> Vec<(usize, usize)> {
    let before = row
        .find(Some(W), "trPr")
        .and_then(|p| p.find(Some(W), "gridBefore"))
        .and_then(|g| int(g.attribute(Some(W), "val")))
        .unwrap_or(0);
    let mut column = usize::try_from(before).unwrap_or(0);
    let mut out = Vec::new();
    let mut stack: Vec<&Arc<Element>> = row.child_elements().collect();
    stack.reverse();
    while let Some(node) = stack.pop() {
        if is_w(node, "tc") {
            let span = node
                .find(Some(W), "tcPr")
                .and_then(|p| p.find(Some(W), "gridSpan"))
                .and_then(|g| int(g.attribute(Some(W), "val")))
                .unwrap_or(1)
                .max(1);
            let span = usize::try_from(span).unwrap_or(1);
            out.push((column, span));
            column += span;
        } else if !is_w(node, "tr") {
            let below: Vec<&Arc<Element>> = node.child_elements().collect();
            stack.extend(below.into_iter().rev());
        }
    }
    out
}

/// An attribute as Python's `int()` reads it (surrounding white space allowed).
fn int(value: Option<&str>) -> Option<i64> {
    value?.trim().parse().ok()
}

/// Each cell's left and right margins, pt, as the layout model resolves the table at
/// `path` (`_cell_margins`), by `(row, cell)`.
fn cell_margins(facts: &Facts<'_>, path: &str) -> HashMap<(usize, usize), (f64, f64)> {
    let Some(table) = facts.model_table(path) else {
        return HashMap::new();
    };
    let resolved = facts.resolved_table(table);
    #[allow(clippy::cast_precision_loss)]
    resolved
        .rows
        .iter()
        .enumerate()
        .flat_map(|(r, row)| {
            row.cells
                .iter()
                .enumerate()
                .map(move |(k, cell)| ((r, k), (cell.margins[1] as f64 / 20.0, cell.margins[3] as f64 / 20.0)))
        })
        .collect()
}
