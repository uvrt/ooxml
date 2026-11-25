//! Owns: [`LikeTheColumn`], text a call wrote into cells that were empty given the
//! formatting of the same column's text in a row around it (docx-agent's
//! `_like_the_column`, round 58, R126 b; the team's ask, a departure from Word, which gives
//! such text the document default -- measured,
//! `crates/ooxml-oracle/word/probes/row_format_probe.py`, `Empty`): in a cell whose
//! paragraph marks and runs of text carry no run formatting of their own (a revision record
//! aside), each run's properties become a copy of the source cell's first
//! run of text's (its own revision record kept, the source's property change left out).
//! Written untracked, as docx-agent writes it, inside the call's step.
//!
//! Must not: write a cell whose row is no longer the row the call wrote (another writer's
//! change between), or change anything else.

use super::shape::{own_rows, row_cells};
use super::{find_table, write, Written};
use crate::names::{W, W14};
use crate::policy::RevisionPolicy;
use crate::review::xml::insert_in_order;
use crate::set_text::block_key;
use crate::sink::EditSink;
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::at_path;

/// The run properties' children that are revision records.
const RECORDS: [&str; 5] = ["ins", "del", "moveFrom", "moveTo", "rPrChange"];

/// One cell to give its column's formatting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnCopy {
    /// The written row, from 0 among the table's own rows.
    pub row: usize,
    /// Its `w14:paraId` when the call read one: a row there with another is not written.
    pub row_id: Option<String>,
    /// The cell's first grid column.
    pub start: usize,
    /// The row whose cell in the same column gives the formatting.
    pub from: usize,
}

/// Give text written into empty cells their column's formatting, as one command.
#[derive(Debug, Clone)]
pub struct LikeTheColumn {
    /// The table (an id, an alias).
    pub table: ooxml_word_model::address::Address,
    /// The cells.
    pub cells: Vec<ColumnCopy>,
}

impl Command<WordDoc> for LikeTheColumn {
    /// Per cell given, whether it was written.
    type Out = Vec<bool>;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let docx = doc.docx()?;
        find_table(doc, &docx, self.table.as_str()).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Vec<bool>> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        let docx = sink.docx()?;
        let found = find_table(sink.doc(), &docx, self.table.as_str())?;
        sink.read(block_key(&found.part, &found.id));
        let shared = sink.doc().shared_handle();
        let orders = shared.facts.orders();
        let tree = sink
            .doc()
            .tree(&found.part)?
            .ok_or_else(|| EditError::no_part(found.part.clone()))?;
        let element = at_path(tree.root(), &found.path).ok_or_else(|| EditError::no_element(found.part.clone()))?;
        let mut work = Work::of(element, scope_above(tree.root(), &found.path));
        let table = work.root();
        let rows = own_rows(&work, table);
        let mut done = Vec::with_capacity(self.cells.len());
        for copy in &self.cells {
            done.push(apply(&mut work, &rows, copy, orders));
        }
        if !done.iter().any(|d| *d) {
            return Ok(done);
        }
        let new = work.build().ok_or_else(|| EditError::unbuilt(found.part.clone()))?;
        write(&mut sink, &found, &found.id, new, &Written::of(None))?;
        crate::after::after_edit(&mut sink)?;
        Ok(done)
    }
}

/// Whether `node`'s run properties are bare: none, or revision records only.
fn bare(work: &Work, properties: Option<Id>) -> bool {
    properties.is_none_or(|p| {
        work.children(p)
            .iter()
            .all(|c| !work.is_element(*c) || work.is_any(*c, W, &RECORDS))
    })
}

/// The cell of `row` starting at grid column `start`.
fn cell_at(work: &Work, row: Id, start: usize) -> Option<Id> {
    row_cells(work, row)
        .into_iter()
        .find(|p| p.start == start)
        .map(|p| p.cell)
}

/// The cell's paragraphs (its own, not nested).
fn paragraphs(work: &Work, cell: Id) -> Vec<Id> {
    work.children(cell)
        .iter()
        .copied()
        .filter(|c| work.is(*c, W, "p"))
        .collect()
}

/// The paragraph's first run that shows text (`first_text_run`).
fn first_text_run(work: &Work, paragraph: Id) -> Option<Id> {
    let skip = ["moveFrom", "del", "p", "txbxContent"];
    work.iter(paragraph, W, &["r"]).into_iter().find(|run| {
        if work.find(*run, W, "t").is_none() {
            return false;
        }
        let mut at = work.parent(*run);
        while let Some(holder) = at {
            if holder == paragraph {
                return true;
            }
            if work.is_any(holder, W, &skip) {
                return false;
            }
            at = work.parent(holder);
        }
        true
    })
}

fn apply(work: &mut Work, rows: &[Id], copy: &ColumnCopy, orders: &crate::validate::Orders) -> bool {
    let (Some(row), Some(source)) = (rows.get(copy.row).copied(), rows.get(copy.from).copied()) else {
        return false;
    };
    if copy
        .row_id
        .as_deref()
        .is_some_and(|id| work.get(row, Some(W14), "paraId") != Some(id))
    {
        return false;
    }
    let Some(cell) = cell_at(work, row, copy.start) else {
        return false;
    };
    let own = paragraphs(work, cell);
    let runs: Vec<Id> = own
        .iter()
        .flat_map(|p| work.iter(*p, W, &["r"]))
        .filter(|r| work.find(*r, W, "t").is_some())
        .collect();
    let marks_bare = own.iter().all(|p| {
        let mark = work.find(*p, W, "pPr").and_then(|pp| work.find(pp, W, "rPr"));
        bare(work, mark)
    });
    if runs.is_empty() || !marks_bare || !runs.iter().all(|r| bare(work, work.find(*r, W, "rPr"))) {
        return false;
    }
    let sample = cell_at(work, source, copy.start).and_then(|neighbour| {
        paragraphs(work, neighbour)
            .into_iter()
            .find_map(|p| first_text_run(work, p))
    });
    let Some(formatting) = sample.and_then(|s| work.find(s, W, "rPr")) else {
        return false;
    };
    for run in runs {
        let new = work.deep_copy(formatting);
        for child in work.children(new).to_vec() {
            if work.is(child, W, "rPrChange") {
                work.remove(child);
            }
        }
        if let Some(own) = work.find(run, W, "rPr") {
            // a revision record of its own stays
            for child in work.children(own).to_vec() {
                if work.is_element(child) {
                    let kept = work.deep_copy(child);
                    insert_in_order(work, new, kept, orders);
                }
            }
            work.remove(own);
        }
        insert_in_order(work, run, new, orders);
    }
    true
}
