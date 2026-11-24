//! Owns: `check`'s `accept_loses_original` (docx-agent's `tools/originals.accept_loses_original`,
//! round 57, R122): text the file held when opened that a pending tracked deletion this
//! session made takes out of the final view -- one entry per listed change and table (or the
//! body's paragraphs), with the cells and paragraphs counted and a sample.
//!
//! Must not: accept or reject anything, or word the reply (the tool layer gives the fix).
//! Units are found by their place in the part, never by node identity.

use crate::names::W;
use crate::originals::{collapse, shown, Originals};
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::docx::changes::authored_changes_holding;
use ooxml_word_model::docx::tables::cells_at;
use ooxml_word_model::docx::{at_path, find_path, Docx, StoryPart};
use ooxml_word_model::ids::Entry;
use ooxml_xml::{Element, Node};
use std::collections::HashSet;
use std::sync::Arc;

/// Records that take content out of view (a move's source is shown where it went).
const DELETED: [&str; 2] = ["del", "cellDel"];
/// The paragraphs an entry without a table names at most.
const PARAS: usize = 20;
/// The places an entry's sample names at most.
const SAMPLE: usize = 3;

/// The file's text one listed change takes out of the final view, in one table or among the
/// body's paragraphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LostOriginal {
    /// The listed change (`word_changes`' id) holding the deletions, where one does.
    pub change: Option<String>,
    /// That change's author.
    pub author: Option<String>,
    /// The table the cells are in; `None` for the body's paragraphs.
    pub table: Option<String>,
    /// Without a table: the paragraphs, at most 20.
    pub paras: Vec<String>,
    /// How many cells.
    pub cells: usize,
    /// How many paragraphs.
    pub paragraphs: usize,
    /// At most three places (a cell on its table's grid, a paragraph's id) and the text
    /// deleted there, white space collapsed.
    pub sample: Vec<(String, String)>,
}

impl LostOriginal {
    /// What a reject `within` names where no listed change holds the deletions: the table,
    /// else the first paragraph.
    #[must_use]
    pub fn within(&self) -> Option<&str> {
        self.table.as_deref().or(self.paras.first().map(String::as_str))
    }
}

/// A listed change's id and author.
type Change = (String, Option<String>);

/// A cell or a paragraph a deletion record is in.
struct Unit {
    cell: bool,
    path: Vec<u32>,
    element: Arc<Element>,
}

/// `check`'s `accept_loses_original` of `doc`, a version of the document `opened` was when
/// the session opened it: the main story's cells and paragraphs, in the order of the records
/// deleting them.
///
/// # Errors
/// When the version cannot be read.
pub fn accept_loses_original(doc: &WordDoc, opened: Arc<Docx>) -> Result<Vec<LostOriginal>> {
    let docx = doc.docx()?;
    let body = docx.body().ok_or_else(|| EditError::no_part(docx.main.clone()))?;
    let mut originals = Originals::new(opened, doc);
    let mut records = Vec::new();
    deletions(body.root(), &mut Vec::new(), &mut Vec::new(), &mut records);
    records.retain(|(_, record, _)| originals.session_record(&body.part, record));
    if records.is_empty() {
        return Ok(Vec::new());
    }
    let units = units(body, &records);
    let mut out: Vec<LostOriginal> = Vec::new();
    for unit in &units {
        let Some((text, change)) = lost(&docx, body, &mut originals, unit) else {
            continue;
        };
        let (at, table) = place(body, unit);
        let key = (change.as_ref().map(|c| c.0.clone()), table);
        let same = |e: &LostOriginal| e.change == key.0 && e.table == key.1;
        if !out.iter().any(same) {
            out.push(LostOriginal {
                change: key.0.clone(),
                author: change.and_then(|c| c.1),
                table: key.1.clone(),
                paras: Vec::new(),
                cells: 0,
                paragraphs: 0,
                sample: Vec::new(),
            });
        }
        let Some(entry) = out.iter_mut().find(|e| same(e)) else {
            continue;
        };
        if unit.cell {
            entry.cells += 1;
        } else {
            entry.paragraphs += 1;
        }
        if entry.table.is_none() && entry.paras.len() < PARAS {
            entry.paras.push(at.clone());
        }
        if entry.sample.len() < SAMPLE {
            entry.sample.push((at, text));
        }
    }
    Ok(out)
}

/// The deletion records at or under `element` (at `at`), each with its place and the
/// places and names of the elements above it.
type Found = (Vec<u32>, Arc<Element>, Vec<(Vec<u32>, &'static str)>);

fn deletions(
    element: &Arc<Element>,
    at: &mut Vec<u32>,
    above: &mut Vec<(Vec<u32>, &'static str)>,
    out: &mut Vec<Found>,
) {
    let name = element.name();
    let word = name.namespace() == Some(W);
    if word && DELETED.contains(&name.local()) {
        out.push((at.clone(), Arc::clone(element), above.clone()));
    }
    let kind = if word {
        match name.local() {
            "tc" => Some("tc"),
            "p" => Some("p"),
            "tr" => Some("tr"),
            "body" => Some("body"),
            _ => None,
        }
    } else {
        None
    };
    if let Some(kind) = kind {
        above.push((at.clone(), kind));
    }
    for (k, node) in element.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        at.push(u32::try_from(k).unwrap_or(u32::MAX));
        deletions(child, at, above, out);
        at.pop();
    }
    if kind.is_some() {
        above.pop();
    }
}

/// The cells and paragraphs the records are in, each once, in the records' order: a
/// paragraph in a cell is its cell, a row's record its cells.
fn units(body: &StoryPart, records: &[Found]) -> Vec<Unit> {
    let mut out = Vec::new();
    let mut seen: HashSet<Vec<u32>> = HashSet::new();
    let mut add = |cell: bool, path: Vec<u32>, out: &mut Vec<Unit>| {
        if seen.insert(path.clone()) {
            if let Some(element) = at_path(body.root(), &path) {
                out.push(Unit {
                    cell,
                    path,
                    element: Arc::clone(element),
                });
            }
        }
    };
    for (_, _, above) in records {
        let Some(k) = above.iter().rposition(|(_, kind)| matches!(*kind, "tc" | "p" | "tr")) else {
            continue;
        };
        let Some((path, kind)) = above.get(k) else { continue };
        match *kind {
            "tr" => {
                let Some(row) = at_path(body.root(), path) else {
                    continue;
                };
                for cell in cells_at(row, path) {
                    add(true, cell.path.to_vec(), &mut out);
                }
            }
            "p" => {
                let cell = above
                    .get(..k)
                    .unwrap_or_default()
                    .iter()
                    .rev()
                    .find(|(_, kind)| matches!(*kind, "tc" | "body"));
                match cell {
                    Some((cell, "tc")) => add(true, cell.clone(), &mut out),
                    _ => add(false, path.clone(), &mut out),
                }
            }
            _ => add(true, path.clone(), &mut out),
        }
    }
    out
}

/// What `unit` loses: the text this session's deletions hold in it and the listed change
/// (its id and author) holding them -- `None` when the unit keeps text in the final view,
/// held none in the file as opened, or is a pending merge's (its text is shown in the
/// merged cell).
fn lost(docx: &Docx, body: &StoryPart, originals: &mut Originals<'_>, unit: &Unit) -> Option<(String, Option<Change>)> {
    if unit.cell && find_path(&unit.element, &["tcPr", "cellMerge"]).is_some() {
        return None;
    }
    if !shown(&unit.element).is_empty() {
        return None;
    }
    let inside = body.ids.paragraphs_within(&unit.path);
    let original = inside
        .iter()
        .filter(|p| unit.cell || *p.path == *unit.path)
        .any(|p| originals.paragraph(&body.part, &p.id));
    if !original {
        return None;
    }
    let mut mine = Vec::new();
    session_records(body, originals, unit, &mut mine);
    let text = deleted_text(&mine);
    if text.is_empty() {
        return None;
    }
    let wanted: Vec<(String, Vec<u32>)> = mine.into_iter().map(|(path, _)| (body.part.clone(), path)).collect();
    let change = authored_changes_holding(docx, &wanted).into_iter().next();
    Some((text, change))
}

/// The deletion records this session wrote that take `unit` out of view: in it, and its
/// row's (a row's deletion is recorded in its `w:trPr`), in document order.
fn session_records(
    body: &StoryPart,
    originals: &mut Originals<'_>,
    unit: &Unit,
    out: &mut Vec<(Vec<u32>, Arc<Element>)>,
) {
    let mut holders = vec![unit.path.clone()];
    let row = (1..unit.path.len())
        .rev()
        .map(|n| unit.path.get(..n).unwrap_or_default())
        .find(|p| at_path(body.root(), p).is_some_and(|e| e.name().is(Some(W), "tr")));
    if let Some(row) = row.and_then(|r| at_path(body.root(), r).map(|e| (r, e))) {
        if find_path(row.1, &["trPr", "del"]).is_some() {
            let k = row
                .1
                .children()
                .iter()
                .position(|n| n.as_element().is_some_and(|e| e.name().is(Some(W), "trPr")));
            if let Some(k) = k.and_then(|k| u32::try_from(k).ok()) {
                holders.push(row.0.iter().copied().chain([k]).collect());
            }
        }
    }
    let mut seen: HashSet<Vec<u32>> = HashSet::new();
    for holder in holders {
        let Some(element) = at_path(body.root(), &holder) else {
            continue;
        };
        let mut found = Vec::new();
        deletions(element, &mut holder.clone(), &mut Vec::new(), &mut found);
        for (path, record, _) in found {
            if !seen.contains(&path) && originals.session_record(&body.part, &record) {
                seen.insert(path.clone());
                out.push((path, record));
            }
        }
    }
}

/// The text the `w:del` records among `records` hold, white space runs as one space
/// (`_deleted_text`).
fn deleted_text(records: &[(Vec<u32>, Arc<Element>)]) -> String {
    let mut out = Vec::new();
    for (_, record) in records {
        if record.name().is(Some(W), "del") {
            let mut text = String::new();
            texts(record, &mut text);
            out.push(text);
        }
    }
    collapse(&out.join(" "))
}

fn texts(element: &Element, out: &mut String) {
    for child in element.child_elements() {
        if child.name().is(Some(W), "delText") || child.name().is(Some(W), "t") {
            out.push_str(&child.text_content());
        } else {
            texts(child, out);
        }
    }
}

/// A unit's place (a cell on its table's grid, a paragraph's id) and its table.
fn place(body: &StoryPart, unit: &Unit) -> (String, Option<String>) {
    if !unit.cell {
        let id = match body.ids.at(&unit.path) {
            Some(Entry::Paragraph(k)) => body.ids.paragraphs().get(k).map(|p| p.id),
            _ => None,
        };
        return (id.unwrap_or_else(|| "?".to_owned()), None);
    }
    let table = (1..unit.path.len())
        .rev()
        .map(|n| unit.path.get(..n).unwrap_or_default())
        .find(|p| at_path(body.root(), p).is_some_and(|e| e.name().is(Some(W), "tbl")));
    let Some(Entry::Table(k)) = table.and_then(|t| body.ids.at(t)) else {
        return ("?".to_owned(), None);
    };
    let Some(table) = body.ids.tables().get(k) else {
        return ("?".to_owned(), None);
    };
    for (r, (row, _)) in table.rows.iter().enumerate() {
        let Some(element) = at_path(body.root(), row) else {
            continue;
        };
        if let Some(cell) = cells_at(element, row).into_iter().find(|c| *c.path == *unit.path) {
            return (format!("{}/c{r},{}", table.id, cell.column), Some(table.id.clone()));
        }
    }
    (table.id.clone(), Some(table.id.clone()))
}
