//! Owns: the texts a check of numbers reads in a Word version (docx-agent's
//! `textfacts.number_texts`): the body's paragraphs, one text each, and each table cell's
//! paragraphs as one text (`t:X/c1,2`, joined by `\n`), in document order, as `find_text`
//! reads them -- deleted text left out, a note's or a drawing's mark read as a space -- each
//! holding a digit; the spans of each written by reference (the version's text facts); and
//! the page each starts on, from the session's layout of the version.
//!
//! Left out: a table of contents' paragraphs, the paragraphs of a table placed from data
//! (its record keeps the values: a table named by a record of the version's), the headers,
//! footers and notes; with `range`, every paragraph outside it (as `check` resolves it);
//! with `pages`, every text on none of them.
//!
//! Must not: read the numbers in a text (the host's tool layer does), or lay out but through
//! the layout given.

use super::{cell_of, lay_out, not_laid, pages_failed, pages_refused, without_pages};
use crate::pages::WordLayout;
use ooxml_base::pystr::is_digit;
use ooxml_base::Cancel;
use ooxml_tools::ToolError;
use ooxml_word_edit::{toc_paragraphs, WordDoc};
use ooxml_word_model::address::direct;
use ooxml_word_model::docx::provenance::{provenance, text_records};
use ooxml_word_model::docx::text::OBJECT;
use ooxml_word_model::docx::{Docx, StoryPart};
use ooxml_word_model::ids::Entry;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// One text a check of numbers reads (`ooxml_edit.tools.NumberText`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberText {
    /// Its address: a paragraph's id, or a cell's (`t:X/c1,2`).
    pub at: String,
    /// Its text, a cell's paragraphs joined by `\n`.
    pub text: String,
    /// The `(start, end)` character spans of `text` written by reference.
    pub written: Vec<(usize, usize)>,
    /// The page it starts on, where the version was laid out.
    pub page: Option<usize>,
}

/// What [`number_texts`] read: the texts, and what the reader is warned of.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NumberTexts {
    /// The texts, in document order.
    pub texts: Vec<NumberText>,
    /// The warnings (a layout that failed where no `pages` were asked for).
    pub warnings: Vec<String>,
}

/// A text found before the layout: its address, text, spans and paragraphs' ids.
struct Found {
    at: String,
    text: String,
    written: Vec<(usize, usize)>,
    ids: Vec<String>,
}

/// The texts a check of numbers reads in the version `doc` (read as `docx`, named `name` in
/// the host's words): within `range` (`check`'s, refs resolved), on `pages` (1-based), each
/// placed by `layout` where one is given.
///
/// # Errors
/// A range refused as `check` refuses it; `pages` asked for where the layout failed;
/// `internal` when a part is not XML.
pub fn number_texts(
    doc: &WordDoc,
    docx: &Arc<Docx>,
    name: &str,
    range: Option<&[String]>,
    pages: Option<&[usize]>,
    layout: Option<&dyn WordLayout>,
    cancel: &dyn Cancel,
) -> Result<NumberTexts, ToolError> {
    let ids = doc.ids();
    let scope = crate::check::scope_of(range, &|a| ids.follow_address(a), docx, name)?;
    let found = read(doc, docx, scope.as_ref().map(|s| &s.paragraphs))?;
    let mut out = NumberTexts::default();
    let laid = match lay_out(layout, docx, cancel) {
        Some(Ok(laid)) => Some(laid),
        Some(Err(why)) => {
            if pages.is_some() {
                let instead = without_pages(name, range, &[("include", serde_json::json!(["numbers"]))], &[]);
                return Err(pages_refused("numbers", why, instead));
            }
            let (code, _) = not_laid(why);
            out.warnings
                .push(format!("numbers: the layout failed ({code}): no pages said"));
            None
        }
        None if pages.is_some() => {
            return Err(pages_failed(
                "numbers",
                "layout_failed",
                "no layout of this document here",
            ));
        }
        None => None,
    };
    for text in found {
        let mut page = None;
        if let Some(laid) = &laid {
            let on = laid.of_paragraphs(text.ids.iter().map(String::as_str));
            if pages.is_some_and(|wanted| !on.iter().any(|p| wanted.contains(p))) {
                continue;
            }
            page = on.first().copied();
        }
        out.texts.push(NumberText {
            at: text.at,
            text: text.text,
            written: text.written,
            page,
        });
    }
    Ok(out)
}

fn internal(error: impl std::fmt::Display) -> ToolError {
    ToolError::internal(error.to_string(), "number_texts")
}

/// What a check of numbers leaves out: a table of contents' paragraphs (each one's place)
/// and the tables placed from data (each one's place).
type LeftOut = (HashSet<Vec<u32>>, Vec<Box<[u32]>>);

/// The places a check of numbers leaves out of the body `body`: a table of contents'
/// paragraphs, and the tables placed from data (named by a record of the version's).
fn left_out(doc: &WordDoc, docx: &Docx, body: &StoryPart) -> Result<LeftOut, ToolError> {
    let toc: HashSet<Vec<u32>> = toc_paragraphs(doc, docx, &body.part).into_iter().collect();
    let mut placed: Vec<Box<[u32]>> = Vec::new();
    for (key, _) in provenance(docx).map_err(internal)?.records {
        if !key.rsplit('/').next().is_some_and(|last| last.starts_with("t:")) {
            continue;
        }
        if let Some((part, Entry::Table(k))) = direct(docx, &key) {
            if part.part == body.part {
                if let Some(table) = part.ids.tables().get(k) {
                    placed.push(table.path);
                }
            }
        }
    }
    Ok((toc, placed))
}

/// A text before it is read for digits: a top-level paragraph's, or a cell's paragraphs'.
struct Group {
    /// The cell's id (`Some(None)`: a cell without one), `None` for a paragraph.
    cell: Option<Option<String>>,
    /// Each paragraph's id and text.
    members: Vec<(String, String)>,
}

/// The body's texts, grouped, in document order: each paragraph's text as `find_text`
/// reads it (an object read as a space), those left out or outside `held` skipped.
fn groups(
    doc: &WordDoc,
    docx: &Docx,
    body: &StoryPart,
    held: Option<&HashSet<String>>,
) -> Result<Vec<Group>, ToolError> {
    let (toc, placed) = left_out(doc, docx, body)?;
    let texts = body.texts();
    let mut out: Vec<Group> = Vec::new();
    let mut by_key: HashMap<Box<[u32]>, usize> = HashMap::new();
    for paragraph in body.ids.paragraphs().iter() {
        if toc.contains(&*paragraph.path) || placed.iter().any(|t| paragraph.path.starts_with(t)) {
            continue;
        }
        if held.is_some_and(|held| !held.contains(&paragraph.id)) {
            continue;
        }
        let text = texts.get(paragraph.index).unwrap_or_default().replace(OBJECT, " ");
        let (key, cell) = match cell_of(body, &paragraph.path) {
            Some(cell) => (cell.path, Some(cell.id)),
            None => (paragraph.path.clone(), None),
        };
        let k = *by_key.entry(key).or_insert_with(|| {
            out.push(Group {
                cell,
                members: Vec::new(),
            });
            out.len() - 1
        });
        if let Some(group) = out.get_mut(k) {
            group.members.push((paragraph.id, text));
        }
    }
    Ok(out)
}

/// The body's texts with a digit, in document order (`number_texts` before the layout).
fn read(doc: &WordDoc, docx: &Docx, held: Option<&HashSet<String>>) -> Result<Vec<Found>, ToolError> {
    let Some(body) = docx.body() else {
        return Ok(Vec::new());
    };
    let records = text_records(docx, doc.ids().aliases()).map_err(internal)?;
    let written: HashMap<&str, &[Value]> = records
        .facts
        .iter()
        .map(|(id, facts)| (id.as_str(), facts.as_slice()))
        .collect();
    let mut out = Vec::new();
    for Group { cell, members } in groups(doc, docx, body, held)? {
        let text = members.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n");
        if !text.chars().any(is_digit) {
            continue;
        }
        let at = match cell {
            None => members.first().map(|(id, _)| id.clone()),
            Some(id) => id,
        };
        let Some(at) = at else { continue };
        let mut marked = Vec::new();
        let mut offset = 0;
        for (id, own) in &members {
            for fact in written.get(id.as_str()).copied().unwrap_or_default() {
                let start = fact
                    .get("at")
                    .and_then(Value::as_u64)
                    .and_then(|a| usize::try_from(a).ok());
                let length = fact
                    .get("text")
                    .and_then(Value::as_str)
                    .map_or(0, |t| t.chars().count());
                if let Some(start) = start {
                    marked.push((offset + start, offset + start + length));
                }
            }
            offset += own.chars().count() + 1;
        }
        out.push(Found {
            at,
            text,
            written: marked,
            ids: members.into_iter().map(|(id, _)| id).collect(),
        });
    }
    Ok(out)
}
