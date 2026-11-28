//! Owns: `check`'s `empty_paragraphs` (docx-agent's `layout_facts.empty_paragraphs`, round
//! 18 Q4): in the body and in each table cell of the main story, every run of empty
//! paragraphs with a paragraph of text on both sides -- blank lines used for spacing -- and
//! the `word_delete` targets that remove them.
//!
//! Must not: lay out (a paragraph's page break before is its formatting, not its page).

use super::py_space;
use crate::names::{M, W};
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::docx::block_facts::BlockFact;
use ooxml_word_model::docx::cells::PartCells;
use ooxml_word_model::docx::formatting::Formatting;
use ooxml_word_model::docx::tables::cells_at;
use ooxml_word_model::docx::text::leading_text;
use ooxml_word_model::docx::{at_path, find_path, StoryPart};
use ooxml_word_model::ids::PlacedTable;
use ooxml_xml::Element;
use std::sync::Arc;

/// The empty paragraphs of one container used as space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyRun {
    /// `body`, or a cell (`<table id>/c<row>,<grid column>`).
    pub at: String,
    /// Their positions among the container's paragraphs, from 0.
    pub paragraphs: Vec<usize>,
    /// Their ids.
    pub ids: Vec<String>,
    /// What `word_delete` takes to remove them: a span where a run's paragraphs are
    /// siblings, else each paragraph, the last first; the last run first.
    pub targets: Vec<String>,
}

/// Run content that shows although it is not text (docx-agent's `_VISIBLE`), in `w:`.
const VISIBLE: [&str; 18] = [
    "drawing",
    "pict",
    "object",
    "br",
    "cr",
    "sym",
    "tab",
    "ptab",
    "fldChar",
    "fldSimple",
    "instrText",
    "footnoteReference",
    "endnoteReference",
    "commentReference",
    "sdt",
    "noBreakHyphen",
    "softHyphen",
    "pgNum",
];

impl EmptyRun {
    /// It, of the container `at`.
    fn at(self, at: String) -> EmptyRun {
        EmptyRun { at, ..self }
    }
}

/// What a paragraph is in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// It shows something.
    Text,
    /// It shows nothing.
    Empty,
    /// It ends a run without being a neighbour: a container's edge, or an empty paragraph
    /// that carries a section break, a frame or a page break before.
    Kept,
}

/// One item of a container's sequence.
enum Item<'a> {
    /// A table, a custom XML block, or either side of a block-level content control.
    Edge,
    /// A paragraph, with its place in the part.
    Paragraph(&'a Arc<Element>, Vec<u32>),
}

/// `check`'s `empty_paragraphs` of `doc`: the body's entry first, then each table's cells
/// in document order (nested tables too).
///
/// # Errors
/// When the version cannot be read.
pub fn empty_paragraphs(doc: &WordDoc) -> Result<Vec<EmptyRun>> {
    let docx = doc.docx()?;
    let body = docx.body().ok_or_else(|| EditError::no_part(docx.main.clone()))?;
    let formatting = Formatting::read(&docx);
    let mut reading = Reading {
        part: body,
        formatting: &formatting,
        cells: PartCells::of(&docx, body),
    };
    let mut found = Vec::new();
    let root = body.root();
    let at_body = root
        .children()
        .iter()
        .position(|n| n.as_element().is_some_and(|e| e.name().is(Some(W), "body")));
    if let Some(k) = at_body.and_then(|k| u32::try_from(k).ok()) {
        if let Some(element) = at_path(root, &[k]) {
            let spaced = reading.spaced(&sequence(element, &[k]));
            found.extend(named(body, &[], &spaced).map(|run| run.at("body".to_owned())));
        }
    }
    // Each table's cells as the block tier keeps them (the table, the outermost table holding
    // it and the formatting parts unchanged); the table's and paragraphs' ids named only for
    // an entry.
    for table in body.ids.tables().placed() {
        let cells = body.block_fact::<TableCells>(&formatting, table.element, &table.path, || {
            Arc::new(reading.cells(&table))
        });
        for cell in cells.iter() {
            if let Some(run) = named(body, &table.path, &cell.spaced) {
                let id = body.ids.tables().head(table.ordinal).map(|t| t.id).unwrap_or_default();
                found.push(run.at(format!("{id}/c{},{}", cell.row, cell.column)));
            }
        }
    }
    Ok(found)
}

/// One run of empty paragraphs between paragraphs of text in a container: their positions
/// among the container's paragraphs, their places (child indexes, below the table holding
/// them or from the root in the body), and whether they stand one after the other.
#[derive(Debug, Clone)]
struct Spaced {
    positions: Vec<usize>,
    places: Vec<Box<[u32]>>,
    siblings: bool,
}

impl Spaced {
    fn weight(&self) -> usize {
        std::mem::size_of::<Spaced>()
            + self.positions.len() * std::mem::size_of::<usize>()
            + self
                .places
                .iter()
                .map(|p| std::mem::size_of::<Box<[u32]>>() + p.len() * 4)
                .sum::<usize>()
    }
}

/// A table cell (the first of a vertical merge) with runs of empty paragraphs used as space.
#[derive(Debug, Clone)]
struct SpacedCell {
    /// Its row among the table's own rows, from 0.
    row: usize,
    /// The grid column it starts at.
    column: usize,
    spaced: Vec<Spaced>,
}

/// A table's cells with runs of empty paragraphs, as the block tier keeps them
/// ([`BlockFact`]): read from the table, the outermost table holding it (a paragraph's cell)
/// and the formatting parts alone, by places below the table.
struct TableCells;

impl BlockFact for TableCells {
    type Value = Arc<Vec<SpacedCell>>;

    fn weight(value: &Self::Value) -> usize {
        std::mem::size_of::<Vec<SpacedCell>>()
            + value
                .iter()
                .map(|c| std::mem::size_of::<SpacedCell>() + c.spaced.iter().map(Spaced::weight).sum::<usize>())
                .sum::<usize>()
    }
}

/// The entry of a container's runs `spaced`, their places below `base`: their paragraphs'
/// ids, and what `word_delete` takes to remove them (docx-agent's `_empty_runs`; its `at`
/// the caller's to name); none without runs.
fn named(part: &StoryPart, base: &[u32], spaced: &[Spaced]) -> Option<EmptyRun> {
    let mut entry = EmptyRun {
        at: String::new(),
        paragraphs: Vec::new(),
        ids: Vec::new(),
        targets: Vec::new(),
    };
    let mut place = base.to_vec();
    for run in spaced {
        let mine: Vec<String> = run
            .places
            .iter()
            .map(|p| {
                place.truncate(base.len());
                place.extend_from_slice(p);
                part.ids.paragraph_at(&place).map(|p| p.id).unwrap_or_default()
            })
            .collect();
        let found: Vec<String> = match (mine.first(), mine.last()) {
            (Some(first), Some(last)) if mine.len() > 1 && run.siblings => vec![format!("{first}..{last}")],
            _ => mine.iter().rev().cloned().collect(),
        };
        entry.targets.splice(0..0, found); // the last run first
        entry.paragraphs.extend(run.positions.iter().copied());
        entry.ids.extend(mine);
    }
    (!entry.ids.is_empty()).then_some(entry)
}

/// A container's paragraphs in order (`at` its place), an edge for a table and around a
/// block-level content control's paragraphs (which are counted).
fn sequence<'a>(container: &'a Element, at: &[u32]) -> Vec<Item<'a>> {
    let mut out = Vec::new();
    for (k, child) in container.children().iter().enumerate() {
        let Some(child) = child.as_element().filter(|c| c.name().namespace() == Some(W)) else {
            continue;
        };
        let place = |k: usize| [at, &[u32::try_from(k).unwrap_or(u32::MAX)]].concat();
        match child.name().local() {
            "p" => out.push(Item::Paragraph(child, place(k))),
            "sdt" => {
                out.push(Item::Edge);
                let content = child
                    .children()
                    .iter()
                    .enumerate()
                    .find(|(_, n)| n.as_element().is_some_and(|e| e.name().is(Some(W), "sdtContent")));
                if let Some((j, content)) = content.and_then(|(j, n)| n.as_element().map(|e| (j, e))) {
                    out.extend(sequence(
                        content,
                        &[place(k).as_slice(), &[u32::try_from(j).unwrap_or(u32::MAX)]].concat(),
                    ));
                }
                out.push(Item::Edge);
            }
            "tbl" | "customXml" => out.push(Item::Edge),
            _ => {}
        }
    }
    out
}

/// Whether `node` or anything below it shows: content of [`VISIBLE`], math, or text that is
/// not white space (deleted text counts).
fn shows(node: &Element) -> bool {
    let name = node.name();
    let here = match name.namespace() {
        Some(W) => {
            let local = name.local();
            VISIBLE.contains(&local)
                || ((local == "t" || local == "delText") && !leading_text(node).chars().all(py_space))
        }
        Some(M) => matches!(name.local(), "oMath" | "oMathPara"),
        _ => false,
    };
    here || node.child_elements().any(|c| shows(c))
}

/// One part's reading for the facts: its paragraphs' ids and cells, its formatting.
struct Reading<'a> {
    part: &'a StoryPart,
    formatting: &'a Formatting,
    cells: PartCells<'a>,
}

impl Reading<'_> {
    /// A paragraph's state (docx-agent's `_state`); `None` where its mark is a tracked
    /// deletion or a move away (the paragraph is going: passed over).
    fn state(&mut self, paragraph: &Arc<Element>, place: &[u32]) -> Option<State> {
        let properties = paragraph.find(Some(W), "pPr");
        let mark = properties.and_then(|p| p.find(Some(W), "rPr"));
        if mark.is_some_and(|m| {
            m.child_elements()
                .any(|c| c.name().is(Some(W), "del") || c.name().is(Some(W), "moveFrom"))
        }) {
            return None;
        }
        let own = paragraph
            .child_elements()
            .filter(|c| !properties.is_some_and(|p| Arc::ptr_eq(p, c)));
        for child in own {
            if shows(child) {
                return Some(State::Text);
            }
        }
        if properties.is_some_and(|p| p.find(Some(W), "sectPr").is_some() || p.find(Some(W), "framePr").is_some()) {
            return Some(State::Kept);
        }
        let cell = self.cells.cell(place);
        if self.formatting.paragraph(paragraph, cell.as_ref()).page_break_before {
            return Some(State::Kept);
        }
        Some(State::Empty)
    }

    /// The cells of `table` (its own rows; a vertical merge's first cell) with runs of empty
    /// paragraphs, by places below the table.
    fn cells(&mut self, table: &PlacedTable<'_>) -> Vec<SpacedCell> {
        let mut out = Vec::new();
        let base = table.path.len();
        for (r, path) in table.rows.iter().enumerate() {
            let Some(row) = at_path(self.part.root(), path) else {
                continue;
            };
            for cell in cells_at(row, path) {
                let merge = find_path(&cell.element, &["tcPr", "vMerge"]);
                if merge.is_none_or(|m| m.attribute(Some(W), "val") == Some("restart")) {
                    let mut spaced = self.spaced(&sequence(&cell.element, &cell.path));
                    for run in &mut spaced {
                        for place in &mut run.places {
                            *place = place.get(base..).unwrap_or_default().into();
                        }
                    }
                    if !spaced.is_empty() {
                        out.push(SpacedCell {
                            row: r,
                            column: cell.column,
                            spaced,
                        });
                    }
                }
            }
        }
        out
    }

    /// The runs of empty paragraphs between paragraphs of text in one container's
    /// `sequence` (docx-agent's `_empty_runs`), by places from the root.
    fn spaced(&mut self, sequence: &[Item<'_>]) -> Vec<Spaced> {
        let mut states: Vec<(usize, Option<&[u32]>, State)> = Vec::new();
        let mut position = 0;
        for item in sequence {
            match item {
                Item::Edge => states.push((usize::MAX, None, State::Kept)),
                Item::Paragraph(element, place) => {
                    if let Some(state) = self.state(element, place) {
                        states.push((position, Some(place.as_slice()), state));
                    }
                    position += 1;
                }
            }
        }
        let mut out = Vec::new();
        let state = |k: usize| states.get(k).map(|s| s.2);
        let mut k = 0;
        while k < states.len() {
            if state(k) != Some(State::Empty) {
                k += 1;
                continue;
            }
            let start = k;
            while state(k) == Some(State::Empty) {
                k += 1;
            }
            if start == 0 || k == states.len() || state(start - 1) != Some(State::Text) || state(k) != Some(State::Text)
            {
                continue;
            }
            let run = states.get(start..k).unwrap_or_default();
            let places: Vec<&[u32]> = run.iter().filter_map(|s| s.1).collect();
            out.push(Spaced {
                positions: run.iter().map(|s| s.0).collect(),
                siblings: places.len() > 1 && self.siblings(&places),
                places: places.into_iter().map(Box::from).collect(),
            });
        }
        out
    }

    /// Whether the paragraphs at `places` stand one after the other among the first's
    /// parent's paragraphs and tables.
    fn siblings(&self, places: &[&[u32]]) -> bool {
        let Some((own, parent_place)) = places.first().and_then(|p| p.split_last()) else {
            return false;
        };
        let Some(parent) = at_path(self.part.root(), parent_place) else {
            return false;
        };
        let blocks: Vec<u32> = parent
            .children()
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                n.as_element()
                    .is_some_and(|e| e.name().is(Some(W), "p") || e.name().is(Some(W), "tbl"))
            })
            .filter_map(|(k, _)| u32::try_from(k).ok())
            .collect();
        let Some(at) = blocks.iter().position(|k| k == own) else {
            return false;
        };
        places.iter().enumerate().all(|(n, p)| {
            p.split_last()
                .is_some_and(|(k, up)| up == parent_place && blocks.get(at + n) == Some(k))
        })
    }
}
