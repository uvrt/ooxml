//! Owns: the body's tables as drawn ([`DrawnTables`], docx-agent's `_drawn_rows`: each
//! outermost table's rows' lines by page, and the pages each table draws a line or a rule
//! on), and the facts read from them ([`tables`]): `split_rows` (a row drawn on two pages)
//! and `header_not_repeated` (a table on more than one page whose header row is not drawn
//! again, or which marks none while its first row looks like one: [`header_look`]).
//!
//! Must not: word an entry (the tool layer does) or cap a list.

use super::tree::{self, is_w, row_of, W};
use super::Facts;
use ooxml_word_model::docx::cells::PartCells;
use ooxml_word_model::docx::text::{shown_runs, View};
use ooxml_word_model::docx::{at_path, StoryPart};
use ooxml_word_model::model::{Block, Table};
use ooxml_word_model::resolve::{resolve_table, Sources};
use ooxml_word_model::value::Value;
use ooxml_xml::Element;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// One row's lines on each page it is drawn on: `(paragraph path, line number)`.
type RowLines = BTreeMap<usize, BTreeSet<(String, usize)>>;

/// The body's outermost tables as drawn, in the order first drawn.
#[derive(Debug, Default)]
pub struct DrawnTables {
    /// Each table's path, in the order first drawn.
    order: Vec<String>,
    /// Each table's rows (from 0), each with its lines by 1-based page.
    rows: HashMap<String, BTreeMap<usize, RowLines>>,
    /// The 1-based pages each table draws a line or a rule on.
    pages: HashMap<String, BTreeSet<usize>>,
}

impl DrawnTables {
    fn page(&mut self, table: &str, page: usize) {
        if !self.pages.contains_key(table) {
            self.order.push(table.to_owned());
        }
        self.pages.entry(table.to_owned()).or_default().insert(page);
    }

    /// Each table's path and pages, in the order first drawn.
    pub fn tables(&self) -> impl Iterator<Item = (&str, &BTreeSet<usize>)> + '_ {
        self.order.iter().filter_map(|t| Some((t.as_str(), self.pages.get(t)?)))
    }

    /// The rows of the table at `path`, from 0, each with its lines by page.
    #[must_use]
    pub fn rows(&self, path: &str) -> Option<&BTreeMap<usize, RowLines>> {
        self.rows.get(path)
    }
}

/// The body's tables as drawn: every body line of a table's row (whether or not the
/// version names its paragraph), and the rules of a page that draws a row.
#[must_use]
pub fn drawn_tables(facts: &Facts<'_>) -> DrawnTables {
    let mut out = DrawnTables::default();
    for (index, page) in facts.version().facts().pages.iter().enumerate() {
        let number = index + 1;
        for line in page.lines.iter().filter(|l| super::story_kind(&l.path) == "body") {
            if let Some((table, row)) = row_of(&line.path) {
                out.rows
                    .entry(table.to_owned())
                    .or_default()
                    .entry(row)
                    .or_default()
                    .entry(number)
                    .or_default()
                    .insert((line.path.to_string(), line.number));
                out.page(table, number);
            }
        }
        if !page.blocks.iter().any(|key| row_of(key).is_some()) {
            continue; // no table drawn: nothing painted to read its rules
        }
        for rule in facts.boxes(index).map(|b| b.rules.as_slice()).unwrap_or_default() {
            if let Some((table, _)) = row_of(&rule.path) {
                out.page(table, number);
            }
        }
    }
    out
}

/// A row (from 0) whose content is drawn on two pages or more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitRow {
    /// The table's id.
    pub table: String,
    /// The row.
    pub row: usize,
    /// The 1-based pages.
    pub pages: Vec<usize>,
}

/// A table on more than one page whose header row is not drawn again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unrepeated {
    /// It marks no header row; its first row looks like one (`looks` says why: then marking
    /// one fixes it), or it has other than two columns (then it is worth marking one only
    /// where its first row is a header: `fix_if_header`).
    Unmarked {
        /// The table's id.
        table: String,
        /// Its 1-based pages.
        pages: Vec<usize>,
        /// Why its first row looks like a header ([`header_look`]).
        looks: Vec<&'static str>,
    },
    /// Its marked header is not drawn on some of its later pages.
    Missing {
        /// The table's id.
        table: String,
        /// Its 1-based pages.
        pages: Vec<usize>,
        /// The rows it marks as its header.
        header_rows: usize,
        /// The later pages its first row is not drawn on.
        not_repeated_on: Vec<usize>,
    },
}

/// `split_rows` and `header_not_repeated` of the tables drawn on more than one page, on the
/// pages asked for (docx-agent's `tables`).
#[must_use]
pub fn tables(facts: &Facts<'_>, drawn: &DrawnTables) -> (Vec<SplitRow>, Vec<Unrepeated>) {
    let (mut split, mut unrepeated) = (Vec::new(), Vec::new());
    let Some(body) = facts.docx().body() else {
        return (split, unrepeated);
    };
    let empty = BTreeMap::new();
    for (table, pages) in drawn.tables() {
        if pages.len() < 2 || !facts.shown(pages.iter().copied()) {
            continue;
        }
        let id = facts.version().id_of(table).unwrap_or(table).to_owned();
        let element = tree::resolve(body, table);
        let header_rows = element.as_ref().map_or(0, |(e, _)| header_rows(e));
        let ordered: Vec<usize> = pages.iter().copied().collect();
        let rows = drawn.rows(table).unwrap_or(&empty);
        for (&row, lines) in rows {
            if lines.len() < 2 {
                continue;
            }
            let mut sets = lines.values();
            let first = sets.next();
            let repeated = row < header_rows && sets.all(|s| Some(s) == first);
            if !repeated && facts.shown(lines.keys().copied()) {
                split.push(SplitRow {
                    table: id.clone(),
                    row,
                    pages: lines.keys().copied().collect(),
                });
            }
        }
        let head = rows.get(&0);
        let missing: Vec<usize> = ordered
            .iter()
            .skip(1)
            .copied()
            .filter(|p| head.is_none_or(|h| !h.contains_key(p)))
            .collect();
        match element {
            Some((element, place)) if header_rows == 0 => {
                let looks = header_look(facts, body, table, element, &place);
                if !looks.is_empty() || columns(element) != 2 {
                    unrepeated.push(Unrepeated::Unmarked {
                        table: id,
                        pages: ordered,
                        looks,
                    });
                }
            }
            _ if !missing.is_empty() => unrepeated.push(Unrepeated::Missing {
                table: id,
                pages: ordered,
                header_rows,
                not_repeated_on: missing,
            }),
            _ => {}
        }
    }
    (split, unrepeated)
}

/// The leading rows a table marks as its header (`w:tblHeader` not off).
fn header_rows(table: &Element) -> usize {
    table
        .child_elements()
        .filter(|e| is_w(e, "tr"))
        .take_while(|row| {
            row.find(Some(W), "trPr")
                .and_then(|p| p.find(Some(W), "tblHeader"))
                .is_some_and(|h| {
                    !matches!(
                        h.attribute(Some(W), "val").unwrap_or("1").to_lowercase().as_str(),
                        "0" | "false" | "off"
                    )
                })
        })
        .count()
}

/// A table's grid columns (its widest row's cells where it has no grid: `_columns`).
pub(crate) fn columns(table: &Element) -> usize {
    let grid = table
        .find(Some(W), "tblGrid")
        .map_or(0, |g| g.child_elements().filter(|e| is_w(e, "gridCol")).count());
    if grid > 0 {
        return grid;
    }
    table
        .child_elements()
        .filter(|e| is_w(e, "tr"))
        .map(|row| row.child_elements().filter(|e| is_w(e, "tc")).count())
        .max()
        .unwrap_or(0)
}

/// The layout model's table at `path` among `blocks`, nested blocks searched too.
pub(crate) fn model_table<'b>(blocks: &'b [Block], path: &str) -> Option<&'b Arc<Table>> {
    for block in blocks {
        let Block::Table(table) = block else { continue };
        if &*table.path == path {
            return Some(table);
        }
        for cells in table.rows.iter() {
            for story in cells.iter() {
                if let Some(found) = model_table(story, path) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// What the layout model's tables resolve against.
pub(crate) fn sources<'v>(facts: &Facts<'v>) -> Sources<'v> {
    let context = &facts.version().layout().document.context;
    Sources {
        styles: context.styles.as_deref(),
        numbering: &context.numbering,
        settings: &context.settings,
    }
}

/// Why a table's first row looks like a header, drawn unlike its second row (docx-agent's
/// `header_look`, round 24, W4): `first_row_look` (its table style's first-row formatting
/// applies to it), `bold` (all its text bold, the second row's not), `shading` (a fill of
/// its own in every cell, unlike the second row's); empty for a first row drawn like the
/// rest.
#[must_use]
pub fn header_look(
    facts: &Facts<'_>,
    story: &StoryPart,
    path: &str,
    element: &Arc<Element>,
    place: &[u32],
) -> Vec<&'static str> {
    let own: Vec<(usize, &Arc<Element>)> = element
        .children()
        .iter()
        .enumerate()
        .filter_map(|(k, n)| n.as_element().filter(|e| is_w(e, "tr")).map(|e| (k, e)))
        .collect();
    if own.len() < 2 {
        return Vec::new();
    }
    let mut reasons = Vec::new();
    let table = model_table(&facts.version().layout().document.body, path);
    let src = sources(facts);
    if let Some(table) = table {
        let first_row = table
            .conditions
            .first()
            .is_some_and(|row| row.iter().any(|cell| cell.iter().any(|c| &**c == "firstRow")));
        if first_row {
            let id = src.table_style_of(table);
            let chain = src
                .styles
                .map(|s| s.chain(id.as_deref(), ooxml_word_model::styles::StyleKind::Table))
                .unwrap_or_default();
            if chain.iter().any(|s| s.conditional.contains_key("firstRow")) {
                reasons.push("first_row_look");
            }
        }
    }
    let mut cells = PartCells::new(facts.formatting(), story.root());
    let mut bold = own.iter().take(2).map(|(k, row)| {
        let mut at = place.to_vec();
        at.push(u32::try_from(*k).unwrap_or(u32::MAX));
        all_bold(facts, &mut cells, row, &at)
    });
    let (first, second) = (bold.next().flatten(), bold.next().flatten());
    if first == Some(true) && second != Some(true) {
        reasons.push("bold");
    }
    if let Some(table) = table.filter(|t| t.rows.len() > 1) {
        let resolved = resolve_table(&src, table);
        let fills: Vec<Vec<Option<String>>> = resolved
            .rows
            .iter()
            .take(2)
            .map(|row| row.cells.iter().map(|c| fill(c.properties.get("shd"))).collect())
            .collect();
        let empty = Vec::new();
        let (first, second) = (fills.first().unwrap_or(&empty), fills.get(1).unwrap_or(&empty));
        if !first.is_empty() && first.iter().all(Option::is_some) && first != second {
            reasons.push("shading");
        }
    }
    reasons
}

/// Whether every run of text in a row is drawn bold (`None`: it has no text): docx-agent's
/// `_all_bold`, each paragraph below the row (nested ones too) and each run in it showing
/// text that is not blank.
fn all_bold(facts: &Facts<'_>, cells: &mut PartCells<'_>, row: &Arc<Element>, place: &[u32]) -> Option<bool> {
    let mut found = None;
    for (paragraph, at) in tree::paragraphs_below(row, place) {
        let cell = cells.cell(&at);
        for run in shown_runs(paragraph, View::Current).into_iter().filter(|r| r.inked) {
            let Some(element) = at_path(paragraph, &run.path) else {
                continue;
            };
            if !facts
                .formatting()
                .run(paragraph, Some(element), run.first, cell.as_ref())
                .bold
            {
                return Some(false);
            }
            found = Some(true);
        }
    }
    found
}

/// A cell's own fill, upper case (`None`: none, `nil` or `auto`): docx-agent's `_fill`.
fn fill(shading: Option<&Value>) -> Option<String> {
    let Some(Value::Shading(shading)) = shading else {
        return None;
    };
    if shading.kind.as_deref() == Some("nil") {
        return None;
    }
    shading.fill.as_deref().filter(|f| *f != "auto").map(str::to_uppercase)
}
