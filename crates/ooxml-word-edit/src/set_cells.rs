//! Owns: [`SetCells`], a table's cells written in one edit as docx-agent's
//! `word_edit_table` writes them (`set_cells`): each cell found by its grid place (a merged
//! cell answering at every place it covers) or by labels -- a row by its first cell's
//! text, a column by the first row's, read as the earlier cells left them (`_locate`) --
//! its first paragraph's text set where it differs (formatting kept), its other paragraphs
//! deleted, each line after the first a paragraph after it -- in order. Unstamped, as
//! `Document._fill_cells` writes them; stamped, as `_set_cell` writes them one by one (the
//! document stamped first where it needs ids).
//!
//! Must not: write but through the sink and the commands it composes, or read outside
//! what they record reading.

use crate::doc::WordDoc;
use crate::policy::RevisionPolicy;
use crate::set_text::block_key;
use crate::sink::EditSink;
use crate::text::write::check_text;
use crate::{EditError, Result};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::direct;
use ooxml_word_model::docx::tables::{placed_row_cells, Accepted, Axis, Grid, RowSpec, TableGrid};
use ooxml_word_model::docx::{at_path, Docx, StoryPart};
use ooxml_word_model::ids::{Entry, TableId};
use ooxml_xml::Element;
use std::sync::Arc;

/// Write a table's cells.
#[derive(Debug, Clone)]
pub struct SetCells {
    /// The table (an id, an alias).
    pub table: String,
    /// Each cell's place and text (`\n` between its paragraphs), in order.
    pub writes: Vec<CellWrite>,
    /// Each cell written as `_set_cell` writes it, the document stamped first where it
    /// needs ids; else as `_fill_cells` writes them, unstamped.
    pub stamping: bool,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
    /// Where given, positions and labels count the table as accepting its tracked changes
    /// leaves it (docx-agent's `_Accepted`, read before the call); else as it is.
    pub accepted: Option<Arc<Accepted>>,
}

/// A cell's row or column for [`SetCells`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellAxis {
    /// The grid row or column, from 0.
    At(usize),
    /// A row by its first cell's text, a column by its first row's (white space collapsed).
    Label(String),
    /// Neither given: refused, at the field docx-agent's `need` names.
    Missing,
}

/// One cell of a [`SetCells`]: where, and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellWrite {
    /// Its row.
    pub row: CellAxis,
    /// Its column.
    pub column: CellAxis,
    /// Its text, `\n` between paragraphs.
    pub text: String,
}

impl CellWrite {
    /// The cell at grid place `(row, column)`.
    #[must_use]
    pub fn at(row: usize, column: usize, text: String) -> CellWrite {
        CellWrite {
            row: CellAxis::At(row),
            column: CellAxis::At(column),
            text,
        }
    }
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
    /// Each write's place, (document row, grid column), in order.
    pub at: Vec<(usize, usize)>,
    /// What positions as accepted stepped over or into, said once each.
    pub notes: Vec<String>,
}

/// A table found: its part, id and rows' places.
struct Table {
    part: String,
    id: String,
    rows: Vec<Box<[u32]>>,
}

/// The table `target` names now, with its story.
pub(crate) fn found_table<'d>(doc: &WordDoc, docx: &'d Docx, target: &str) -> Result<(&'d StoryPart, TableId)> {
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
    Ok((story, found))
}

/// The table `target` names now.
fn table(doc: &WordDoc, docx: &Docx, target: &str) -> Result<Table> {
    let (story, found) = found_table(doc, docx, target)?;
    Ok(Table {
        part: story.part.clone(),
        id: found.id.clone(),
        rows: found.rows.iter().map(|(path, _)| path.clone()).collect(),
    })
}

/// Every grid place's cell, as its first place `(row, column)` (docx-agent's
/// `Table.grid`, by [`Grid`]); `None` where a row has no cell.
pub(crate) fn grid(root: &Arc<Element>, rows: &[Box<[u32]>]) -> Vec<Vec<Option<(usize, usize)>>> {
    let specs: Vec<RowSpec> = rows
        .iter()
        .map(|path| at_path(root, path).map_or_else(|| RowSpec::new(None, None, Vec::new()), |row| RowSpec::of(row)))
        .collect();
    let grid = Grid::build(&specs, 0);
    grid.rows
        .iter()
        .zip(rows)
        .map(|(line, path)| {
            if at_path(root, path).is_none() {
                return Vec::new();
            }
            let mut out = vec![None; line.before];
            for cell in &line.cells {
                let (r, k) = cell.origin;
                let first = grid
                    .rows
                    .get(r)
                    .and_then(|o| o.cells.get(k))
                    .map_or(cell.first, |o| o.first);
                out.extend(std::iter::repeat_n(Some((r, first)), cell.span));
            }
            out
        })
        .collect()
}

/// The place of the `w:tc` of the row at `row` whose grid column is `column`
/// (docx-agent's `cell_element`).
pub(crate) fn cell_path(root: &Arc<Element>, row: &[u32], column: usize) -> Option<Vec<u32>> {
    let element = at_path(root, row)?;
    let (_, below, _) = placed_row_cells(element)
        .into_iter()
        .find(|(_, _, p)| p.first == column)?;
    let mut path = row.to_vec();
    path.extend_from_slice(&below);
    Some(path)
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
        self.write(tx, false)
    }
}

impl SetCells {
    /// The cells written in `tx`, as one part of a command placing a source's rows: a table
    /// with a record of its source written too (that command keeps the record).
    pub(crate) fn prepare_placing(&self, tx: &mut Txn<'_, WordDoc>) -> Result<CellsSet> {
        self.write(tx, true)
    }

    fn write(&self, tx: &mut Txn<'_, WordDoc>, recorded: bool) -> Result<CellsSet> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let docx = sink.docx()?;
        let found = table(sink.doc(), &docx, &self.table)?;
        if !recorded && crate::tables::placed(sink.doc(), &found.id) {
            // docx-agent's `_Kept`: the cells written drop their columns from the record
            // ([`crate::KeepingRecord`] does)
            return Err(EditError::Unsupported(
                "set_cells in a table placed from a source: its record is kept through KeepingRecord".to_owned(),
            ));
        }
        sink.read(block_key(&found.part, &found.id));
        let tree = sink
            .doc()
            .tree(&found.part)?
            .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
        let places = grid(tree.root(), &found.rows);
        let mut out = CellsSet {
            edits: Vec::new(),
            ids: Vec::new(),
            at: Vec::new(),
            notes: Vec::new(),
        };
        for write in &self.writes {
            let (row, column) = match &self.accepted {
                Some(accepted) => located(&sink, &self.table, write, accepted, &mut out.notes)?,
                None => locate(&sink, &self.table, write)?,
            };
            let Some((r, c)) = places.get(row).and_then(|l| l.get(column)).copied().flatten() else {
                let docx = sink.docx()?;
                let (story, now) = found_table(sink.doc(), &docx, &self.table)?;
                return Err(EditError::Address(
                    TableGrid::new(story, &now).no_cell(&now.id, row, column),
                ));
            };
            write_cell(
                &mut sink,
                &found,
                &places,
                (row, column),
                (&write.text, self.stamping),
                &mut out.edits,
            )?;
            out.ids.push(format!("{}/c{r},{c}", found.id));
            out.at.push((row, column));
        }
        // docx-agent's edit, once it ends: text boxes synced, revision ids unique
        sink.space_written()?;
        crate::textbox::sync(&mut sink)?;
        crate::revision_ids::make_unique(&mut sink)?;
        Ok(out)
    }
}

/// A write's grid place (docx-agent's `_locate`): its labels read on the table as the
/// writes before it left it -- the row's in the first column, the column's in the first
/// row.
fn locate(sink: &EditSink<'_, '_>, target: &str, write: &CellWrite) -> Result<(usize, usize)> {
    if let (CellAxis::At(row), CellAxis::At(column)) = (&write.row, &write.column) {
        return Ok((*row, *column));
    }
    let docx = sink.docx()?;
    let (story, found) = found_table(sink.doc(), &docx, target)?;
    let grid = TableGrid::new(story, &found);
    let row = match &write.row {
        CellAxis::At(row) => *row,
        CellAxis::Label(label) => grid.row_labelled(label).map_err(EditError::Address)?,
        CellAxis::Missing => return Err(missing("cells[].row or row_label")),
    };
    let column = match &write.column {
        CellAxis::At(column) => *column,
        CellAxis::Label(label) => grid.column_labelled(0, label).map_err(EditError::Address)?,
        CellAxis::Missing => return Err(missing("cells[].col or col_label")),
    };
    Ok((row, column))
}

/// A write's (document row, grid column) with positions and labels as `accepted` counts
/// them (`_locate` with `accepted`): labels read as the writes before it left the table.
fn located(
    sink: &EditSink<'_, '_>,
    target: &str,
    write: &CellWrite,
    accepted: &Accepted,
    notes: &mut Vec<String>,
) -> Result<(usize, usize)> {
    let index = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
    if let (CellAxis::At(row), CellAxis::At(column)) = (&write.row, &write.column) {
        return accepted
            .place(index(*row), index(*column), "cells[]", notes)
            .map_err(EditError::Position);
    }
    let row = axis(&write.row, "cells[].row or row_label")?;
    let column = axis(&write.column, "cells[].col or col_label")?;
    let docx = sink.docx()?;
    let (story, found) = found_table(sink.doc(), &docx, target)?;
    let grid = TableGrid::new(story, &found);
    accepted
        .locate(&grid, (row, column), notes)
        .map_err(EditError::Position)
}

/// A write's row or column as a position or label; refused at `field` where missing.
fn axis<'w>(axis: &'w CellAxis, field: &str) -> Result<Axis<'w>> {
    match axis {
        CellAxis::At(at) => Ok(Axis::At(i64::try_from(*at).unwrap_or(i64::MAX))),
        CellAxis::Label(label) => Ok(Axis::Label(label.as_str())),
        CellAxis::Missing => Err(missing(field)),
    }
}

/// docx-agent's `need`: `field` is required.
fn missing(field: &str) -> EditError {
    EditError::Argument {
        field: field.to_owned(),
        message: format!("{field} is required"),
        options: Vec::new(),
    }
}

/// docx-agent's `_set_cell` (`word_set_text` on a cell, in lines): the cell at the grid
/// place `(row, column)` of `table` given `text`, its first paragraph stamped and written
/// as set_text writes it where it differs; its edits.
///
/// # Errors
/// A refusal for a target that is not a table or a place it lacks.
pub(crate) fn set_cell(
    sink: &mut EditSink<'_, '_>,
    table_id: &str,
    (row, column): (usize, usize),
    text: &str,
) -> Result<Vec<CellEdit>> {
    let docx = sink.docx()?;
    let found = table(sink.doc(), &docx, table_id)?;
    sink.read(block_key(&found.part, &found.id));
    let tree = sink
        .doc()
        .tree(&found.part)?
        .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
    let places = grid(tree.root(), &found.rows);
    let mut edits = Vec::new();
    write_cell(sink, &found, &places, (row, column), (text, true), &mut edits)?;
    Ok(edits)
}

/// One cell's text (`stamping`: its first paragraph written as an edit of its own, the
/// document stamped first).
fn write_cell(
    sink: &mut EditSink<'_, '_>,
    table: &Table,
    places: &[Vec<Option<(usize, usize)>>],
    (row, column): (usize, usize),
    (text, stamping): (&str, bool),
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
        let wrote = crate::set_text::write_text_as(sink, first, head, stamping)?;
        edits.push(CellEdit {
            id: Some(wrote.id),
            renamed: wrote.renamed,
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
