//! Owns: `check`'s `accept_loses_text` (docx-agent's `layout_facts.accept_loses_text`, round
//! 54 lane R): every body table cell Word's Accept All removes with its text, with the
//! tracked change that does it and the reject that keeps it.
//!
//! Must not: accept or reject anything (the cells are read from the table as it is).

use super::py_space;
use crate::names::W;
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::docx::revisions::records;
use ooxml_word_model::docx::tables::cells_lost_on_accept;
use ooxml_word_model::docx::text::leading_text;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// The longest text an entry shows, in characters.
const SHOWN: usize = 80;

/// A cell Accept All removes with its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LostCell {
    /// The cell (`<table id>/c<row>,<grid column>`).
    pub cell: String,
    /// Its text (white space collapsed, shortened to 80 characters with an ellipsis).
    pub text: String,
    /// The tracked change that deletes its last paragraph mark, as `word_changes` names it.
    pub change: Option<String>,
    /// That change's author.
    pub author: Option<String>,
}

impl LostCell {
    /// The cell's table (`<table id>`).
    #[must_use]
    pub fn table(&self) -> &str {
        self.cell
            .rsplit_once("/c")
            .map_or(self.cell.as_str(), |(table, _)| table)
    }
}

/// `check`'s `accept_loses_text` of `doc`: the main story's tables in document order
/// (nested ones too), each one's cells row by row.
///
/// # Errors
/// When the version cannot be read.
pub fn accept_loses_text(doc: &WordDoc) -> Result<Vec<LostCell>> {
    let docx = doc.docx()?;
    let body = docx.body().ok_or_else(|| EditError::NoPart(docx.main.clone()))?;
    let mut changes: Option<HashMap<Box<[u32]>, String>> = None;
    let mut found = Vec::new();
    let tables = body.ids.tables();
    for placed in tables.placed() {
        let lost = cells_lost_on_accept(body, &placed.rows);
        let Some(table) = (!lost.is_empty()).then(|| tables.head(placed.ordinal)).flatten() else {
            continue;
        };
        for (row, cell) in lost {
            let by_record = changes.get_or_insert_with(|| {
                records(&docx)
                    .into_iter()
                    .filter(|r| r.part == body.part)
                    .map(|r| (r.path, r.id))
                    .collect()
            });
            let mut paragraphs = Vec::new();
            paragraphs_below(&cell.element, &cell.path, &mut paragraphs);
            let record = paragraphs.last().and_then(|(p, at)| mark_record(p, at));
            let texts: Vec<String> = paragraphs.iter().map(|(p, _)| kept_text(p, false)).collect();
            let text = texts
                .join(" ")
                .split(py_space)
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            found.push(LostCell {
                cell: format!("{}/c{row},{}", table.id, cell.column),
                text: shortened(&text),
                change: record
                    .as_ref()
                    .and_then(|(_, at)| by_record.get(at.as_slice()).cloned()),
                author: record
                    .as_ref()
                    .and_then(|(r, _)| r.attribute(Some(W), "author"))
                    .map(str::to_owned),
            });
        }
    }
    Ok(found)
}

/// Every paragraph below `element` (at `at`), with its place, in document order (nested
/// tables' too).
fn paragraphs_below(element: &Element, at: &[u32], out: &mut Vec<(Arc<Element>, Vec<u32>)>) {
    for (k, child) in element.children().iter().enumerate() {
        let Some(child) = child.as_element() else { continue };
        let place = [at, &[u32::try_from(k).unwrap_or(u32::MAX)]].concat();
        if child.name().is(Some(W), "p") {
            out.push((Arc::clone(child), place.clone()));
        }
        paragraphs_below(child, &place, out);
    }
}

/// The deletion or move away of a paragraph's mark (the first such child of `w:pPr/w:rPr`),
/// with its place.
fn mark_record(paragraph: &Element, at: &[u32]) -> Option<(Arc<Element>, Vec<u32>)> {
    let mut place = at.to_vec();
    let mut node = paragraph;
    for local in ["pPr", "rPr"] {
        let (k, child) = node
            .children()
            .iter()
            .enumerate()
            .find_map(|(k, n)| n.as_element().filter(|e| e.name().is(Some(W), local)).map(|e| (k, e)))?;
        place.push(u32::try_from(k).ok()?);
        node = child;
    }
    let (k, record) = node
        .children()
        .iter()
        .enumerate()
        .find_map(|(k, n)| n.as_element().filter(|e| going(e)).map(|e| (k, e)))?;
    place.push(u32::try_from(k).ok()?);
    Some((Arc::clone(record), place))
}

fn going(element: &Element) -> bool {
    element.name().is(Some(W), "del") || element.name().is(Some(W), "moveFrom")
}

/// The text of `element`'s `w:t` outside deletions and moves away (`deleted`: `element`
/// is inside one).
fn kept_text(element: &Element, deleted: bool) -> String {
    let mut out = String::new();
    for child in element.child_elements() {
        if child.name().is(Some(W), "t") && !deleted {
            out.push_str(&leading_text(child));
        }
        out.push_str(&kept_text(child, deleted || going(child)));
    }
    out
}

/// `text` shortened to [`SHOWN`] characters, the last an ellipsis.
fn shortened(text: &str) -> String {
    if text.chars().count() <= SHOWN {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(SHOWN - 1).collect();
    out.push('\u{2026}');
    out
}
