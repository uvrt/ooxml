//! Owns: a table's rows written in a session against the rows around them (docx-agent's
//! `layout_facts.rows_unlike`, round 58, R126): per written row, the cells whose text shows
//! another font, size or colour than the same column's cells in the nearest rows above and
//! below that were not written, with the `word_format` calls that make them match
//! ([`rows_unlike`]); and what a row shows of its text ([`row_shown`]), by which a call
//! tells the rows it wrote.
//!
//! Rows that legitimately differ are not compared: a header row (`w:tblHeader`, or the
//! table's first row), the last row where the table's look has a total row, a column whose
//! rows above and below differ from each other, and a cell whose look an unwritten row has
//! in that column within [`ROW_WINDOW`] rows (banded or alternating rows). A cell shows its
//! first run of text's formatting; an empty cell nothing.
//!
//! Must not: change the document, word a reply or cap entries (the tool layer does), or
//! find an element by its address (equal subtrees share one node): rows and cells are found
//! by their places.

use ooxml_word_model::docx::cells::{model_path, TableFacts};
use ooxml_word_model::docx::formatting::Formatting;
use ooxml_word_model::docx::{at_path, find_path, Docx};
use ooxml_word_model::ids::{TableId, W};
use ooxml_word_model::markdown::read::Shared;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// How far from a written row the rows with its look are looked for (rows in a pattern,
/// banded or alternating, are within so many rows).
pub const ROW_WINDOW: usize = 4;

/// What a cell's text looks like: its first run of text's face, size and colour.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLook {
    /// The face (`None`: none resolved).
    pub font: Option<String>,
    /// Points.
    pub size: f64,
    /// `RRGGBB` or `auto`.
    pub color: String,
}

/// A field of [`TextLook`], in the order `row_text_format` compares them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LookField {
    /// The face (`font`; `word_format`'s `font`).
    Font,
    /// The size (`sz`; `word_format`'s `size`).
    Size,
    /// The colour (`color`; `word_format`'s `color`).
    Color,
}

impl LookField {
    /// The three, in order.
    pub const ALL: [LookField; 3] = [LookField::Font, LookField::Size, LookField::Color];

    /// Its key in a `row_text_format` entry's `has` and `around`.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            LookField::Font => "font",
            LookField::Size => "sz",
            LookField::Color => "color",
        }
    }

    /// `word_format`'s argument for it.
    #[must_use]
    pub fn argument(self) -> &'static str {
        match self {
            LookField::Font => "font",
            LookField::Size => "size",
            LookField::Color => "color",
        }
    }
}

impl TextLook {
    fn differs(&self, other: &TextLook, field: LookField) -> bool {
        match field {
            LookField::Font => self.font != other.font,
            // sizes are half points: exact
            #[allow(clippy::float_cmp)]
            LookField::Size => self.size != other.size,
            LookField::Color => self.color != other.color,
        }
    }
}

/// One cell of a written row unlike the rows around it.
#[derive(Debug, Clone, PartialEq)]
pub struct CellUnlike {
    /// Its id, `<table>/c<row>,<first grid column>`.
    pub cell: String,
    /// Its first grid column.
    pub start: usize,
    /// What its text looks like.
    pub has: TextLook,
    /// What the rows around it share.
    pub around: TextLook,
    /// The fields that differ, in order (`has` and `around` say only these).
    pub fields: Vec<LookField>,
    /// The row `around` was read from: the one above where there is one.
    pub from: usize,
}

/// A `word_format` call that gives cells the formatting around them.
#[derive(Debug, Clone, PartialEq)]
pub struct FormatCall {
    /// The cells.
    pub targets: Vec<String>,
    /// The values, `fields` of it given.
    pub look: TextLook,
    /// The fields given, in order.
    pub fields: Vec<LookField>,
}

/// A written row whose text looks unlike the rows around it.
#[derive(Debug, Clone, PartialEq)]
pub struct RowUnlike {
    /// The table's id.
    pub table: String,
    /// The row, from 0 among the table's own rows.
    pub row: usize,
    /// Its cells unlike their column, in order.
    pub cells: Vec<CellUnlike>,
    /// The calls that make them match, one per set of values.
    pub fix: Vec<FormatCall>,
}

/// The text a row shows: its `w:t`s' text, nested tables' included.
#[must_use]
pub fn row_shown(row: &Element) -> String {
    let mut out = String::new();
    texts(row, &mut out);
    out
}

fn texts(element: &Element, out: &mut String) {
    for child in element.child_elements() {
        if child.name().is(Some(W), "t") {
            out.push_str(&child.text_content());
        } else {
            texts(child, out);
        }
    }
}

/// A row's own cells as `(cell, its place, first grid column, span)`: those inside
/// row-level content controls and custom XML too, not a nested table's.
#[must_use]
pub fn row_cells(row: &Arc<Element>, path: &[u32]) -> Vec<(Arc<Element>, Vec<u32>, usize, usize)> {
    let mut column = int_val(find_path(row, &["trPr", "gridBefore"]), 0);
    let mut found = Vec::new();
    cells_in(row, path, &mut found);
    found
        .into_iter()
        .map(|(cell, place)| {
            let span = int_val(find_path(&cell, &["tcPr", "gridSpan"]), 1).max(1);
            let start = column;
            column += span;
            (cell, place, start, span)
        })
        .collect()
}

fn cells_in(element: &Arc<Element>, path: &[u32], out: &mut Vec<(Arc<Element>, Vec<u32>)>) {
    for (k, node) in element.children().iter().enumerate() {
        let Some(child) = node.as_element() else { continue };
        let mut place = path.to_vec();
        place.push(u32::try_from(k).unwrap_or(u32::MAX));
        if child.name().is(Some(W), "tc") {
            out.push((Arc::clone(child), place));
        } else if !child.name().is(Some(W), "tbl") && !child.name().is(Some(W), "tr") {
            cells_in(child, &place, out);
        }
    }
}

fn int_val(node: Option<&Element>, default: usize) -> usize {
    node.and_then(|n| n.attribute(Some(W), "val"))
        .and_then(|v| v.trim().parse::<i64>().ok())
        .map_or(default, |v| usize::try_from(v).unwrap_or(0))
}

/// Whether the table's look turns its total row on (docx-agent's `read_look`: an attribute
/// where written, else its bit in `w:val`; a table without `w:tblLook` has none).
fn total_row(table: &Element) -> bool {
    let Some(look) = find_path(table, &["tblPr", "tblLook"]) else {
        return false;
    };
    if let Some(value) = look.attribute(Some(W), "lastRow") {
        return matches!(value, "1" | "true" | "on");
    }
    look.attribute(Some(W), "val")
        .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_hexdigit()))
        .and_then(|v| u64::from_str_radix(v, 16).ok())
        .is_some_and(|bits| bits & 0x0040 != 0)
}

/// The paragraph's first run that shows text (`first_text_run`): one with a `w:t`, not in
/// a deletion, a move's source, a nested paragraph or a text box.
fn first_text_run(paragraph: &Element) -> Option<Arc<Element>> {
    fn walk(node: &Element) -> Option<Arc<Element>> {
        for child in node.child_elements() {
            if child.name().is(Some(W), "r") {
                if child.find(Some(W), "t").is_some() {
                    return Some(Arc::clone(child));
                }
                continue;
            }
            if ["moveFrom", "del", "p", "txbxContent"]
                .iter()
                .any(|t| child.name().is(Some(W), t))
            {
                continue;
            }
            if let Some(found) = walk(child) {
                return Some(found);
            }
        }
        None
    }
    walk(paragraph)
}

/// The run's first character (which decides the script, so the face): its text's first,
/// a tab or a break as itself; `a` for none.
fn first_char(run: &Element) -> char {
    for child in run.child_elements() {
        let local = child.name().local();
        if !child.name().is(Some(W), local) {
            continue;
        }
        match local {
            "t" => {
                if let Some(c) = child.text_content().chars().next() {
                    return c;
                }
            }
            "tab" | "ptab" => return '\t',
            "br" | "cr" => return '\n',
            _ => {}
        }
    }
    'a'
}

/// A row's cells, `(first grid column, span)`, with what their text looks like.
type RowLooks = Vec<((usize, usize), Option<TextLook>)>;

/// A table's rows as `rows_unlike` reads them: which are compared, and what each cell's
/// text looks like (read once per cell).
struct Scan<'a> {
    formatting: &'a Formatting,
    facts: Arc<TableFacts>,
    table: &'a TableId,
    /// Each own row with its place (`None`: not found).
    rows: Vec<Option<(Arc<Element>, Vec<u32>)>>,
    marked: &'a [bool],
    /// The table's look turns its total row on.
    total: bool,
    memo: HashMap<usize, RowLooks>,
}

impl Scan<'_> {
    fn header(&self, r: usize) -> bool {
        r == 0
            || self
                .rows
                .get(r)
                .and_then(Option::as_ref)
                .is_some_and(|(row, _)| find_path(row, &["trPr", "tblHeader"]).is_some())
    }

    fn written(&self, r: usize) -> bool {
        self.marked.get(r).copied().unwrap_or(false)
    }

    /// A row the written rows are compared against: not written, not a header, not the
    /// total row.
    fn compared(&self, r: usize) -> bool {
        !self.written(r) && !self.header(r) && !(r + 1 == self.rows.len() && self.total)
    }

    /// The row's cells, `(first grid column, span)`, with their looks, in order.
    fn row(&mut self, r: usize) -> RowLooks {
        if let Some(found) = self.memo.get(&r) {
            return found.clone();
        }
        let mut out = Vec::new();
        if let Some(Some((row, path))) = self.rows.get(r) {
            for (cell, place, start, span) in row_cells(row, path) {
                out.push(((start, span), self.cell(&cell, &place)));
            }
        }
        self.memo.insert(r, out.clone());
        out
    }

    fn get(&mut self, r: usize, key: (usize, usize)) -> Option<TextLook> {
        self.row(r)
            .into_iter()
            .find(|(k, _)| *k == key)
            .and_then(|(_, look)| look)
    }

    /// A cell's text's look: its first run of text's, in its first paragraph with text;
    /// `None` for a cell that shows none.
    fn cell(&self, cell: &Arc<Element>, place: &[u32]) -> Option<TextLook> {
        let mut paragraphs = Vec::new();
        paragraphs_in(cell, place, &mut paragraphs);
        for (paragraph, path) in paragraphs {
            let Some(run) = first_text_run(&paragraph) else {
                continue;
            };
            let below = path.get(self.table.path.len()..).unwrap_or_default();
            let where_ = model_path(&self.table.element, below).and_then(|p| self.facts.cell(&p).cloned());
            let got = self
                .formatting
                .run(&paragraph, Some(&run), first_char(&run), where_.as_ref());
            return Some(TextLook {
                font: got.font,
                size: got.size,
                color: got.color,
            });
        }
        None
    }

    /// Written row `r` against the rows around it, if it is unlike them.
    fn unlike(&mut self, r: usize) -> Option<RowUnlike> {
        let count = self.rows.len();
        let above = (0..r).rev().find(|i| self.compared(*i));
        let below = (r + 1..count).find(|i| self.compared(*i));
        if above.is_none() && below.is_none() {
            return None;
        }
        let mut cells = Vec::new();
        let mut fix: Vec<FormatCall> = Vec::new();
        for (key, has) in self.row(r) {
            let Some(has) = has else { continue };
            let Some(cell) = self.cell_unlike(r, key, has, [above, below]) else {
                continue;
            };
            match fix
                .iter_mut()
                .find(|c| c.fields == cell.fields && !cell.fields.iter().any(|f| c.look.differs(&cell.around, *f)))
            {
                Some(call) => call.targets.push(cell.cell.clone()),
                None => fix.push(FormatCall {
                    targets: vec![cell.cell.clone()],
                    look: cell.around.clone(),
                    fields: cell.fields.clone(),
                }),
            }
            cells.push(cell);
        }
        (!cells.is_empty()).then(|| RowUnlike {
            table: self.table.id.clone(),
            row: r,
            cells,
            fix,
        })
    }

    /// The cell of written row `r` at `key` against the same column's cells in the rows
    /// `around` (above, below), if it is unlike them.
    fn cell_unlike(
        &mut self,
        r: usize,
        key: (usize, usize),
        has: TextLook,
        around: [Option<usize>; 2],
    ) -> Option<CellUnlike> {
        let seen: Vec<(usize, TextLook)> = around
            .into_iter()
            .flatten()
            .filter_map(|i| self.get(i, key).map(|l| (i, l)))
            .collect();
        let (from, expected) = seen.first().cloned()?;
        if seen.get(1).is_some_and(|(_, other)| *other != expected) {
            return None;
        }
        let nearby = r.saturating_sub(ROW_WINDOW)..(r + ROW_WINDOW + 1).min(self.rows.len());
        for i in nearby {
            if self.compared(i) && self.get(i, key).as_ref() == Some(&has) {
                return None; // a look the table's own rows have nearby: banded, alternating
            }
        }
        let fields: Vec<LookField> = LookField::ALL
            .into_iter()
            .filter(|f| has.differs(&expected, *f))
            .collect();
        (!fields.is_empty()).then(|| CellUnlike {
            cell: format!("{}/c{r},{}", self.table.id, key.0),
            start: key.0,
            has,
            around: expected,
            fields,
            from,
        })
    }
}

/// Every `w:p` at or under `element`, in document order, with its place.
fn paragraphs_in(element: &Arc<Element>, path: &[u32], out: &mut Vec<(Arc<Element>, Vec<u32>)>) {
    for (k, node) in element.children().iter().enumerate() {
        let Some(child) = node.as_element() else { continue };
        let mut place = path.to_vec();
        place.push(u32::try_from(k).unwrap_or(u32::MAX));
        if child.name().is(Some(W), "p") {
            out.push((Arc::clone(child), place.clone()));
        }
        paragraphs_in(child, &place, out);
    }
}

/// `table`'s (a table of `part` of `docx`) written rows unlike the rows around them:
/// `marked[r]` says whether its own row `r` was written this session.
#[must_use]
pub fn rows_unlike(docx: &Docx, part: &str, table: &TableId, marked: &[bool]) -> Vec<RowUnlike> {
    let Some(story) = docx.part(part) else {
        return Vec::new();
    };
    let shared = Shared::of(docx);
    let mut scan = Scan {
        formatting: shared.formatting(),
        facts: shared.table_facts(part, table),
        table,
        rows: table
            .rows
            .iter()
            .map(|(path, _)| at_path(story.root(), path).map(|row| (Arc::clone(row), path.to_vec())))
            .collect(),
        marked,
        total: total_row(&table.element),
        memo: HashMap::new(),
    };
    (0..scan.rows.len())
        .filter(|r| scan.written(*r) && !scan.header(*r))
        .collect::<Vec<_>>()
        .into_iter()
        .filter_map(|r| scan.unlike(r))
        .collect()
}
