//! Owns: what a read's `range` selects (docx-agent's `markdown.select` over its address
//! resolver, `edit/addresses.py`): the body (no range), a story by name, a section (`s:2`,
//! `s:body`, `s:<paraId>`), a block (`p:`, `t:`, `p@body/7`, `header1/p:...`, a paragraph
//! in a table names its table), a span of blocks (`p:A..t:B`, either end open), a heading's
//! range (`h:p:A`), a table's row (`t:X/r1`, `tr:...`) or cell (`t:X/c1,2`).
//!
//! Not yet resolved here (refused with a way out; listed in the lane's hand-back):
//! bookmarks (`bm:`), text ranges (`p:A@3:9`), content controls (`cc:`), renamed ids.
//!
//! Must not: render, or change the document.

use crate::docx::controls::controls;
use crate::docx::outline::{body_blocks, headings};
use crate::docx::StoryPart;
use crate::markdown::read::{Reader, Record, TableRecord};

/// How a span is written, in every refusal of one.
pub const SPAN_SYNTAX: &str = "a span is two block ids, <first>..<last> (p:A..t:B); p:A.. runs to the end of p:A's container (the body, a cell), ..t:B from its start";

/// Why a range was refused: its code (`not_found`, `invalid_arguments`), message and ids
/// that would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeError {
    /// `not_found` or `invalid_arguments`.
    pub code: &'static str,
    /// What is wrong, and what to pass.
    pub message: String,
    /// Addresses that would do.
    pub options: Vec<String>,
}

impl RangeError {
    fn not_found(message: String, options: Vec<String>) -> RangeError {
        RangeError {
            code: "not_found",
            message,
            options,
        }
    }

    fn invalid(message: String, options: Vec<String>) -> RangeError {
        RangeError {
            code: "invalid_arguments",
            message,
            options,
        }
    }
}

/// What a range selects.
pub enum Selected {
    /// Records to write; `story` names a story read whole (other than the body).
    Records {
        /// The story's name, for its `story:` comment.
        story: Option<String>,
        /// The records.
        records: Vec<Record>,
    },
    /// Every note of a notes part.
    Notes(String),
}

fn records(story: Option<String>, records: Vec<Record>) -> Selected {
    Selected::Records { story, records }
}

/// The records `range` selects.
///
/// # Errors
/// A [`RangeError`] when it names nothing or is malformed.
pub fn select(reader: &mut Reader<'_>, range: Option<&str>) -> Result<Vec<Selected>, RangeError> {
    let docx = reader.docx;
    let Some(body) = docx.body() else { return Ok(Vec::new()) };
    let Some(range) = range else {
        return Ok(vec![records(None, reader.story(body))]);
    };
    let range = expand_heading(reader, range)?;
    if range.trim().is_empty() {
        return Err(RangeError::invalid(
            format!("a range is empty: give a block id, a span ({SPAN_SYNTAX}), a section or a story"),
            block_options(body),
        ));
    }
    if let Some((first, last)) = span_ends(&range)? {
        let anchor = first.or(last).unwrap_or_default();
        let part = part_of(reader, anchor)?;
        if first.is_some() && last.is_some() && part_of(reader, last.unwrap_or_default())?.part != part.part {
            return Err(RangeError::invalid(
                format!("{range:?} spans two stories: {SPAN_SYNTAX}"),
                Vec::new(),
            ));
        }
        let all = reader.story(part);
        let a = match first {
            Some(f) => position(reader, &all, f)?,
            None => 0,
        };
        let b = match last {
            Some(l) => through(&all, position(reader, &all, l)?),
            None => all.len().saturating_sub(1),
        };
        if b < a {
            return Err(RangeError::invalid(
                format!("'{range}' ends before it starts"),
                Vec::new(),
            ));
        }
        return Ok(vec![records(None, all.into_iter().skip(a).take(b + 1 - a).collect())]);
    }
    if range.starts_with("bm:")
        || range.contains('@') && !range.starts_with("p@") && !range.starts_with("t@")
        || range.starts_with("cc")
    {
        return Err(not_yet(&range));
    }
    if let Some(found) = inside_table(reader, &range)? {
        return Ok(vec![found]);
    }
    if range.starts_with("s:") || range.starts_with("s@") {
        return section(reader, body, &range).map(|r| vec![r]);
    }
    if !range.contains([':', '@', '/']) {
        let Some(story) = docx.story(&range) else {
            if range.contains('#') {
                return Err(not_yet(&range));
            }
            let names: Vec<String> = docx.parts.iter().map(|p| p.story.clone()).collect();
            return Err(RangeError::not_found(format!("no story '{range}'"), names));
        };
        let root = story.root();
        if root.name().is(Some(crate::ids::W), "footnotes") || root.name().is(Some(crate::ids::W), "endnotes") {
            return Ok(vec![Selected::Notes(story.part.clone())]);
        }
        let name = (story.part != body.part).then(|| story.story.clone());
        return Ok(vec![records(name, reader.story(story))]);
    }
    let part = part_of(reader, &range)?;
    let all = reader.story(part);
    let k = position(reader, &all, &range)?;
    let end = through(&all, k);
    Ok(vec![records(None, all.into_iter().skip(k).take(end + 1 - k).collect())])
}

fn not_yet(range: &str) -> RangeError {
    RangeError::invalid(
        format!("'{range}': bookmarks, text ranges and content controls are not read in the Rust core yet; read a block, span, heading, section or story"),
        Vec::new(),
    )
}

fn block_options(part: &StoryPart) -> Vec<String> {
    part.ids.paragraphs.iter().take(10).map(|p| p.id.clone()).collect()
}

/// `h:p:A` as the span its heading owns; a span's `h:` ends likewise.
fn expand_heading(reader: &Reader<'_>, range: &str) -> Result<String, RangeError> {
    if !range.contains("h:") {
        return Ok(range.to_owned());
    }
    let docx = reader.docx;
    let found = headings(&body_blocks(docx, &controls(docx)));
    let one = |end: &str, first: bool| -> Result<String, RangeError> {
        let Some(id) = end.strip_prefix("h:") else {
            return Ok(end.to_owned());
        };
        let options: Vec<String> = found.iter().take(20).map(|h| format!("h:{}", h.id)).collect();
        let heading = found.iter().find(|h| h.id == id).ok_or_else(|| {
            RangeError::invalid(
                format!("{id} is not a heading: h:<heading> is the heading and every block under it"),
                options,
            )
        })?;
        Ok(if first {
            heading.id.clone()
        } else {
            heading.last.clone()
        })
    };
    match range.split_once("..") {
        Some((a, b)) => {
            let first = if a.is_empty() { String::new() } else { one(a, true)? };
            let last = if b.is_empty() { String::new() } else { one(b, false)? };
            Ok(format!("{first}..{last}"))
        }
        None => {
            let first = one(range, true)?;
            let last = one(range, false)?;
            Ok(if first == last {
                first
            } else {
                format!("{first}..{last}")
            })
        }
    }
}

/// The two ends of a span of blocks, or `None` when `range` is not one.
fn span_ends(range: &str) -> Result<Option<(Option<&str>, Option<&str>)>, RangeError> {
    if !range.contains("..") {
        return Ok(None);
    }
    let parts: Vec<&str> = range.split("..").collect();
    let ends = match parts.as_slice() {
        [a, b] if !(a.is_empty() && b.is_empty()) && !a.ends_with('.') && !b.starts_with('.') => (*a, *b),
        _ => {
            return Err(RangeError::invalid(
                format!("'{range}' is not a span: {SPAN_SYNTAX}"),
                Vec::new(),
            ))
        }
    };
    if ends.0.contains('@') && !ends.0.starts_with("p@") || ends.1.contains('@') && !ends.1.starts_with("p@") {
        return Ok(None);
    }
    Ok(Some((non_empty(ends.0.trim()), non_empty(ends.1.trim()))))
}

/// The story part an id is in (docx-agent's `_direct`).
fn part_of<'d>(reader: &Reader<'d>, id: &str) -> Result<&'d StoryPart, RangeError> {
    let docx = reader.docx;
    let story = if let Some(rest) = id.strip_prefix("p@").or_else(|| id.strip_prefix("t@")) {
        rest.rsplit_once('/').map_or("", |(s, _)| s)
    } else if let Some((s, _)) = id.split_once('/') {
        s
    } else {
        "body"
    };
    let part = docx.story(story).filter(|p| p.ids.get(id).is_some());
    part.ok_or_else(|| {
        let options = docx.body().map(block_options).unwrap_or_default();
        RangeError::not_found(format!("no block '{id}'"), options)
    })
}

/// The index of the top-level record holding `id`.
fn position(reader: &Reader<'_>, all: &[Record], id: &str) -> Result<usize, RangeError> {
    let _ = reader;
    all.iter()
        .position(|r| holds(r, id))
        .ok_or_else(|| RangeError::not_found(format!("'{id}' is not a block of its story in this view"), Vec::new()))
}

fn holds(record: &Record, id: &str) -> bool {
    match record {
        Record::Marker(m) => m.id == id,
        Record::Paragraph(p) => p.id == id || p.joins.iter().any(|j| j == id),
        Record::Table(t) => t.id == id || t.rows.iter().flatten().any(|c| c.records.iter().any(|r| holds(r, id))),
    }
}

/// The record closing the block at `k`: a control's end for its start.
fn through(all: &[Record], k: usize) -> usize {
    if let Some(Record::Marker(m)) = all.get(k) {
        if m.begin {
            let found = all
                .iter()
                .enumerate()
                .skip(k + 1)
                .find(|(_, r)| matches!(r, Record::Marker(e) if !e.begin && e.id == m.id));
            if let Some((j, _)) = found {
                return j;
            }
        }
    }
    k
}

/// A row (its table with that row alone) or a cell (its blocks).
fn inside_table(reader: &mut Reader<'_>, address: &str) -> Result<Option<Selected>, RangeError> {
    let (table_id, row, column) = if let Some(rest) = address.strip_prefix("tr:").map(|r| format!("tr:{r}")) {
        let Some(part) = reader
            .docx
            .parts
            .iter()
            .find(|p| p.ids.tables.iter().any(|t| t.rows.iter().any(|(_, r)| *r == rest)))
        else {
            return Err(RangeError::not_found(format!("no row '{address}'"), Vec::new()));
        };
        let table = part.ids.tables.iter().find(|t| t.rows.iter().any(|(_, r)| *r == rest));
        let Some(table) = table else { return Ok(None) };
        let index = table.rows.iter().position(|(_, r)| *r == rest).unwrap_or(0);
        (table.id.clone(), index, None)
    } else if let Some((table, inside)) = address.rsplit_once('/') {
        if let Some(cell) = inside.strip_prefix('c') {
            let Some((r, c)) = cell.split_once(',') else {
                return Ok(None);
            };
            let (Ok(r), Ok(c)) = (r.parse::<usize>(), c.parse::<usize>()) else {
                return Ok(None);
            };
            (table.to_owned(), r, Some(c))
        } else if let Some(r) = inside.strip_prefix('r').and_then(|r| r.parse::<usize>().ok()) {
            (table.to_owned(), r, None)
        } else {
            return Ok(None);
        }
    } else {
        return Ok(None);
    };
    let part = part_of(reader, &table_id)?;
    let all = reader.story(part);
    let found = find_table(&all, &table_id).cloned();
    let Some(mut table) = found else {
        return Err(RangeError::not_found(
            format!("'{address}' is not in its story in this view"),
            Vec::new(),
        ));
    };
    match column {
        None => {
            table.rows.retain(|line| line.first().is_some_and(|c| c.row == row));
            table.header_rows = 0;
            Ok(Some(Selected::Records {
                story: None,
                records: vec![Record::Table(Box::new(table))],
            }))
        }
        Some(column) => {
            let cell = table.rows.iter().flatten().find(|c| {
                c.row <= row && row < c.row + c.row_span && c.column <= column && column < c.column + c.column_span
            });
            match cell {
                Some(cell) => Ok(Some(Selected::Records {
                    story: None,
                    records: cell.records.clone(),
                })),
                None => Err(RangeError::not_found(
                    format!("no cell at ({row}, {column}) in {table_id}"),
                    Vec::new(),
                )),
            }
        }
    }
}

fn find_table<'r>(records: &'r [Record], id: &str) -> Option<&'r TableRecord> {
    for record in records {
        if let Record::Table(t) = record {
            if t.id == id {
                return Some(t);
            }
            for cell in t.rows.iter().flatten() {
                if let Some(found) = find_table(&cell.records, id) {
                    return Some(found);
                }
            }
        }
    }
    None
}

fn section(reader: &mut Reader<'_>, body: &StoryPart, range: &str) -> Result<Selected, RangeError> {
    let all = reader.story(body);
    let ends: Vec<usize> = all
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r, Record::Paragraph(p) if p.section.is_some()))
        .map(|(k, _)| k)
        .collect();
    let found = crate::docx::sections::sections(reader.docx);
    if ends.is_empty() && found.is_empty() && (range == "s:body" || range == "s:1") {
        return Ok(records(None, all));
    }
    let wanted = match range
        .strip_prefix("s:")
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| *n >= 1 && *n < 10_000)
    {
        Some(n) => found.get(n - 1).map(|s| s.id.clone()),
        None => found.iter().find(|s| s.id == range).map(|s| s.id.clone()),
    };
    let Some(wanted) = wanted else {
        let options: Vec<String> = found.iter().map(|s| s.id.clone()).collect();
        return Err(RangeError::not_found(format!("no section '{range}'"), options));
    };
    if wanted == "s:body" {
        let start = ends.last().map_or(0, |k| k + 1);
        return Ok(records(None, all.into_iter().skip(start).collect()));
    }
    for (n, k) in ends.iter().enumerate() {
        let id = match all.get(*k) {
            Some(Record::Paragraph(p)) => p.section.as_ref().map(|s| s.0.clone()),
            _ => None,
        };
        if id.as_deref() == Some(wanted.as_str()) {
            let start = if n > 0 { ends.get(n - 1).map_or(0, |e| e + 1) } else { 0 };
            return Ok(records(None, all.into_iter().skip(start).take(k + 1 - start).collect()));
        }
    }
    Err(RangeError::not_found(format!("no section '{range}'"), Vec::new()))
}

fn non_empty(s: &str) -> Option<&str> {
    (!s.is_empty()).then_some(s)
}
