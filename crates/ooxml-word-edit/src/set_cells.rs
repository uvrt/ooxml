//! Owns: [`SetCells`], a table's cells written in one pass as docx-agent's
//! `Document._fill_cells` writes them (`word_edit_table`'s `set_cells`): each cell found by
//! its grid place (a merged cell answering at every place it covers), its first
//! paragraph's text set where it differs (formatting kept, unstamped), its other
//! paragraphs deleted, each line after the first a paragraph after it -- in order, one
//! edit.
//!
//! Must not: write but through the sink and the commands it composes, or read outside
//! what they record reading.

use crate::doc::WordDoc;
use crate::names::W;
use crate::policy::RevisionPolicy;
use crate::set_text::block_key;
use crate::sink::EditSink;
use crate::text::write::check_text;
use crate::{EditError, Result};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::direct;
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_word_model::ids::Entry;
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// Write a table's cells.
#[derive(Debug, Clone)]
pub struct SetCells {
    /// The table (an id, an alias).
    pub table: String,
    /// Each cell's grid row, grid column and text (`\n` between its paragraphs), in order.
    pub writes: Vec<(usize, usize, String)>,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// One edit a set_cells made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellEdit {
    /// The paragraph written or made; `None` for a deletion.
    pub id: Option<String>,
    /// Ids renamed, old to new.
    pub renamed: Vec<(String, String)>,
    /// Paragraphs made.
    pub created: Vec<String>,
    /// Paragraphs removed.
    pub removed: Vec<String>,
}

/// What a set_cells did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellsSet {
    /// Its edits, in order.
    pub edits: Vec<CellEdit>,
    /// Each write's cell, `<table>/c<row>,<column>` (the merged cell's first place).
    pub ids: Vec<String>,
}

/// A table found: its part, id and rows' places.
struct Table {
    part: String,
    id: String,
    rows: Vec<Box<[u32]>>,
}

/// The table `target` names now.
fn table(doc: &WordDoc, docx: &Docx, target: &str) -> Result<Table> {
    let followed = doc.ids().follow(target);
    let block = (followed != target)
        .then(|| direct(docx, &followed))
        .flatten()
        .or_else(|| direct(docx, target));
    let Some((story, Entry::Table(k))) = block else {
        return Err(EditError::Refused(format!("{target} is not a table")));
    };
    let found = story
        .ids
        .tables()
        .get(k)
        .ok_or_else(|| EditError::Refused(format!("{target} is not a table")))?;
    Ok(Table {
        part: story.part.clone(),
        id: found.id.clone(),
        rows: found.rows.iter().map(|(path, _)| path.clone()).collect(),
    })
}

/// A whole-number property (`w:val` of the child at `steps`), or `default`.
fn number(element: &Element, steps: &[&str], default: usize) -> usize {
    ooxml_word_model::docx::find_path(element, steps)
        .and_then(|e| e.attribute(Some(W), "val"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

/// A row's `w:tc`, also inside row-level `w:sdt` and `w:customXml`, each with its place
/// below the row (docx-agent's `_cells`).
fn cells<'e>(element: &'e Element, at: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, &'e Arc<Element>)>) {
    for (k, node) in element.children().iter().enumerate() {
        let (Node::Element(child), Ok(k)) = (node, u32::try_from(k)) else {
            continue;
        };
        at.push(k);
        if child.name().is(Some(W), "tc") {
            out.push((at.clone(), child));
        } else if child.name().is(Some(W), "sdt") {
            if let Some((c, content)) = child.children().iter().enumerate().find_map(|(c, n)| {
                n.as_element()
                    .filter(|e| e.name().is(Some(W), "sdtContent"))
                    .map(|e| (c, e))
            }) {
                at.push(u32::try_from(c).unwrap_or(u32::MAX));
                cells(content, at, out);
                at.pop();
            }
        } else if child.name().is(Some(W), "customXml") {
            cells(child, at, out);
        }
        at.pop();
    }
}

/// Every grid place's cell, as its first place `(row, column)` (docx-agent's
/// `Table.grid`); `None` where a row has no cell.
fn grid(root: &Arc<Element>, rows: &[Box<[u32]>]) -> Vec<Vec<Option<(usize, usize)>>> {
    let mut origins: HashMap<usize, (usize, usize)> = HashMap::new();
    let mut out = Vec::new();
    for (r, path) in rows.iter().enumerate() {
        let mut line: Vec<Option<(usize, usize)>> = Vec::new();
        let Some(row) = at_path(root, path) else {
            out.push(line);
            continue;
        };
        let before = number(row, &["trPr", "gridBefore"], 0);
        line.extend(std::iter::repeat_n(None, before));
        let mut column = before;
        let mut found = Vec::new();
        cells(row, &mut Vec::new(), &mut found);
        for (_, tc) in found {
            let span = number(tc, &["tcPr", "gridSpan"], 1).max(1);
            let merge = ooxml_word_model::docx::find_path(tc, &["tcPr", "vMerge"]);
            let continues = merge.is_some_and(|m| matches!(m.attribute(Some(W), "val"), None | Some("continue")));
            let origin = match continues.then(|| origins.get(&column).copied()).flatten() {
                Some(origin) => origin,
                None => {
                    for c in column..column + span {
                        origins.insert(c, (r, column));
                    }
                    (r, column)
                }
            };
            line.extend(std::iter::repeat_n(Some(origin), span));
            column += span;
        }
        out.push(line);
    }
    out
}

/// The place of the `w:tc` of the row at `row` whose grid column is `column`
/// (docx-agent's `cell_element`).
fn cell_path(root: &Arc<Element>, row: &[u32], column: usize) -> Option<Vec<u32>> {
    let element = at_path(root, row)?;
    let mut at = number(element, &["trPr", "gridBefore"], 0);
    let mut found = Vec::new();
    cells(element, &mut Vec::new(), &mut found);
    for (below, tc) in found {
        if at == column {
            let mut path = row.to_vec();
            path.extend(below);
            return Some(path);
        }
        at += number(tc, &["tcPr", "gridSpan"], 1).max(1);
    }
    None
}

/// The ids of the paragraphs of the cell at `path`, in document order.
fn cell_paragraphs(docx: &Docx, part: &str, path: &[u32]) -> Vec<String> {
    docx.part(part)
        .map(|story| story.ids.paragraphs_within(path).into_iter().map(|p| p.id).collect())
        .unwrap_or_default()
}

impl Command<WordDoc> for SetCells {
    type Out = CellsSet;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        table(doc, &*doc.docx()?, &self.table).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<CellsSet> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let docx = sink.docx()?;
        let found = table(sink.doc(), &docx, &self.table)?;
        sink.read(block_key(&found.part, &found.id));
        let tree = sink
            .doc()
            .tree(&found.part)?
            .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
        let places = grid(tree.root(), &found.rows);
        let mut out = CellsSet {
            edits: Vec::new(),
            ids: Vec::new(),
        };
        for (row, column, text) in &self.writes {
            write_cell(&mut sink, &found, &places, (*row, *column), text, &mut out.edits)?;
            if let Some(Some((r, c))) = places.get(*row).and_then(|l| l.get(*column)) {
                out.ids.push(format!("{}/c{r},{c}", found.id));
            }
        }
        // docx-agent's edit, once it ends: text boxes synced, revision ids unique
        crate::textbox::sync(&mut sink)?;
        crate::revision_ids::make_unique(&mut sink)?;
        Ok(out)
    }
}

/// One cell's text.
fn write_cell(
    sink: &mut EditSink<'_, '_>,
    table: &Table,
    places: &[Vec<Option<(usize, usize)>>],
    (row, column): (usize, usize),
    text: &str,
    edits: &mut Vec<CellEdit>,
) -> Result<()> {
    let missing = || EditError::Refused(format!("{} has no cell at row {row}, column {column}", table.id));
    let (r, c) = places
        .get(row)
        .and_then(|l| l.get(column))
        .copied()
        .flatten()
        .ok_or_else(missing)?;
    let tree = sink
        .doc()
        .tree(&table.part)?
        .ok_or_else(|| EditError::NoPart(table.part.clone()))?;
    let row_path = table.rows.get(r).ok_or_else(missing)?;
    let path = cell_path(tree.root(), row_path, c).ok_or_else(missing)?;
    let docx = sink.docx()?;
    let paragraphs = cell_paragraphs(&docx, &table.part, &path);
    let (first, extra) = paragraphs.split_first().ok_or_else(missing)?;
    let lines: Vec<&str> = text.split('\n').collect();
    let head = lines.first().copied().unwrap_or_default();
    let current = docx
        .part(&table.part)
        .and_then(|s| s.ids.paragraphs_within(&path).into_iter().next())
        .map(|p| {
            let work = crate::work::Work::of(&p.element, Vec::new());
            crate::text::paragraph_text(&work, work.root())
        })
        .unwrap_or_default();
    if current != head {
        check_text(head)?;
        let wrote = crate::set_text::write_text_as(sink, first, head, false)?;
        edits.push(CellEdit {
            id: Some(wrote.id),
            renamed: Vec::new(),
            created: Vec::new(),
            removed: Vec::new(),
        });
    }
    for id in extra {
        let gone = crate::delete_write::delete(sink, id)?;
        edits.push(CellEdit {
            id: gone.id,
            renamed: gone.renamed,
            created: Vec::new(),
            removed: gone.removed,
        });
    }
    if lines.len() > 1 {
        let docx = sink.docx()?;
        let mut last = cell_paragraphs(&docx, &table.part, &path)
            .into_iter()
            .next()
            .ok_or_else(missing)?;
        for line in lines.iter().skip(1) {
            let made = crate::paragraphs::insert_paragraph(sink, &last, (true, line, None))?;
            edits.push(CellEdit {
                id: Some(made.id.clone()),
                renamed: made.renamed,
                created: vec![made.id.clone()],
                removed: Vec::new(),
            });
            last = made.id;
        }
    }
    Ok(())
}
