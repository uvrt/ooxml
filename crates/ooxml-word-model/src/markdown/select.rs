//! Owns: what a read's `range` selects (docx-agent's `markdown.select`), every form the
//! address resolver ([`crate::address`]) reads: the body (no range), a story by name, a
//! section, a block (a paragraph in a table names its table), a span of blocks (either end
//! open), a heading's range, a row (its table with that row alone), a cell (its blocks), a
//! bookmark or a text range (the blocks holding it), a content control.
//!
//! A block, span, bookmark or text range of a body, header or footer is read alone: the
//! blocks before it counted for their list items only, those after it not read
//! ([`super::read::Groups`]); the records are then cut exactly as docx-agent cuts the
//! story's.
//!
//! Must not: render, or change the document.

use crate::address::resolve::{in_table, InTable};
use crate::address::{self, span_ends, AddressError};
use crate::docx::controls::controls;
use crate::docx::StoryPart;
use crate::ids::{Entry, W};
use crate::markdown::read::{Reader, Record, TableRecord};
use ooxml_base::pyfmt::repr_str;

pub use crate::address::SPAN_SYNTAX;

/// Why a range was refused (the address resolver's refusal).
pub type RangeError = AddressError;

/// What a range selects.
pub enum Selected {
    /// Records to write.
    Records {
        /// The story's name, for its `story:` comment, where a story other than the body is
        /// read whole.
        story: Option<String>,
        /// The records.
        records: Vec<Record>,
        /// Whether they are one cell's blocks.
        cell: bool,
    },
    /// Every note of a notes part.
    Notes(String),
}

fn records(story: Option<String>, records: Vec<Record>) -> Selected {
    Selected::Records {
        story,
        records,
        cell: false,
    }
}

/// How the records of a story are cut.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cut {
    /// A span: from the first's record through the last's (a control's end with it).
    Span,
    /// From whichever of two records comes first through the other (a text range, a
    /// bookmark).
    Between,
    /// One block's record (a control's through its end).
    Block,
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
    let range = address::expand_heading(docx, range)?;
    let range = range.as_str();
    if range.trim().is_empty() {
        return Err(AddressError::invalid(
            format!("a range is empty: give a block id, a span ({SPAN_SYNTAX}), a section or a story"),
            Vec::new(),
        ));
    }
    if let Some((first, last)) = span_ends(range)? {
        let anchor = first.or(last).unwrap_or_default();
        let part = part_of(reader, anchor)?;
        if let (Some(_), Some(last)) = (first, last) {
            if part_of(reader, last)?.part != part.part {
                return Err(AddressError::invalid(
                    format!("{} spans two stories: {SPAN_SYNTAX}", repr_str(range)),
                    Vec::new(),
                ));
            }
        }
        return cut(reader, part, first, last, Cut::Span, range).map(|r| vec![records(None, r)]);
    }
    if address::is_text_range(range) {
        let found = address::text_range(docx, range)?;
        return between(reader, &found.start_id, &found.end_id);
    }
    if range.starts_with("bm:") || !range.contains([':', '@', '/', '#']) && docx.story(range).is_none() {
        match address::bookmark(docx, range) {
            Ok(found) => {
                let found = address::bookmark_range(docx, &found)?;
                return between(reader, &found.start_id, &found.end_id);
            }
            Err(error) if range.starts_with("bm:") => return Err(error),
            Err(_) => {}
        }
    }
    if let Some(found) = inside_table(reader, range)? {
        return Ok(vec![found]);
    }
    if range.starts_with("s:") || range.starts_with("s@") {
        return section(reader, body, range).map(|r| vec![r]);
    }
    if !range.contains([':', '@', '/']) {
        let Some(story) = docx.story(range) else {
            return Err(AddressError::not_found(
                format!("no story {}", repr_str(range)),
                Vec::new(),
            ));
        };
        let root = story.root();
        if root.name().is(Some(W), "footnotes") || root.name().is(Some(W), "endnotes") {
            return Ok(vec![Selected::Notes(story.part.clone())]);
        }
        let name = (story.part != body.part).then(|| story.story.clone());
        return Ok(vec![records(name, reader.story(story))]);
    }
    let part = part_of(reader, range)?;
    cut(reader, part, Some(range), None, Cut::Block, range).map(|r| vec![records(None, r)])
}

/// The blocks from the one holding paragraph `first` through the one holding `last`.
fn between(reader: &mut Reader<'_>, first: &str, last: &str) -> Result<Vec<Selected>, RangeError> {
    let part = part_of(reader, first)?;
    cut(reader, part, Some(first), Some(last), Cut::Between, first).map(|r| vec![records(None, r)])
}

/// The story part an id is in (docx-agent's `_part_of`).
fn part_of<'d>(reader: &Reader<'d>, id: &str) -> Result<&'d StoryPart, RangeError> {
    let docx = reader.docx;
    if id.starts_with("cc") {
        let found = controls(docx).into_iter().find(|c| c.id == id);
        return found
            .and_then(|c| docx.part(&c.part))
            .ok_or_else(|| AddressError::not_found(format!("no content control {}", repr_str(id)), Vec::new()));
    }
    Ok(address::resolve(docx, id)?.0)
}

/// A block's place in its part, or a control's id.
type Place = (Option<Box<[u32]>>, Option<String>);

/// Where a block id's element is in its part; for a control, its id (its markers carry it).
fn place(reader: &Reader<'_>, part: &StoryPart, id: &str) -> Result<Place, RangeError> {
    if id.starts_with("cc") {
        return Ok((None, Some(id.to_owned())));
    }
    let (_, entry) = address::resolve(reader.docx, id)?;
    let path = match entry {
        Entry::Paragraph(k) => part.ids.paragraphs().get(k).map(|p| p.path.clone()),
        Entry::Table(k) => part.ids.tables().get(k).map(|t| t.path.clone()),
    };
    Ok((path, None))
}

/// The records `how` cuts from `part`'s story for `first` and `last` (`None`: open). A
/// container's story is read only over the blocks the cut can reach.
fn cut(
    reader: &mut Reader<'_>,
    part: &StoryPart,
    first: Option<&str>,
    last: Option<&str>,
    how: Cut,
    given: &str,
) -> Result<Vec<Record>, RangeError> {
    let all = match reader.groups_of(part) {
        None => reader.story(part),
        Some(groups) if groups.is_empty() => Vec::new(),
        Some(groups) => {
            let position = |reader: &Reader<'_>, id: &str| -> Result<usize, RangeError> {
                let (path, control) = place(reader, part, id)?;
                groups
                    .position(path.as_deref(), control.as_deref())
                    .ok_or_else(|| not_in_view(id))
            };
            let a = first.map(|f| position(reader, f)).transpose()?.unwrap_or(0);
            let b = match (last, how) {
                (Some(l), _) => position(reader, l)?,
                (None, Cut::Block) => a,
                (None, _) => groups.len() - 1,
            };
            let (low, high) = (a.min(b), groups.through(a.max(b)));
            reader.read_window(groups, low..=high)
        }
    };
    let (a, b) = match how {
        Cut::Span => {
            let a = first.map(|f| position(reader, &all, f)).transpose()?.unwrap_or(0);
            let b = match last {
                Some(l) => through(&all, position(reader, &all, l)?),
                None => all.len().saturating_sub(1),
            };
            if b < a {
                return Err(AddressError::refused(format!(
                    "{} ends before it starts",
                    repr_str(given)
                )));
            }
            (a, b)
        }
        Cut::Between => {
            let a = position(reader, &all, first.unwrap_or_default())?;
            let b = position(reader, &all, last.unwrap_or_default())?;
            (a.min(b), a.max(b))
        }
        Cut::Block => {
            let k = position(reader, &all, first.unwrap_or_default())?;
            (k, through(&all, k))
        }
    };
    Ok(all.into_iter().skip(a).take(b + 1 - a).collect())
}

fn not_in_view(id: &str) -> RangeError {
    AddressError::not_found(
        format!("{} is not a block of its story in this view", repr_str(id)),
        Vec::new(),
    )
}

/// The index of the top-level record holding `id` (its own id, as `_canonical` gives it).
fn position(reader: &Reader<'_>, all: &[Record], id: &str) -> Result<usize, RangeError> {
    let wanted = if id.starts_with("cc") {
        id.to_owned()
    } else {
        let (part, entry) = address::resolve(reader.docx, id)?;
        part.ids.id_of(entry).unwrap_or_else(|| id.to_owned())
    };
    all.iter()
        .position(|r| holds(r, &wanted))
        .ok_or_else(|| not_in_view(id))
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
    let Some(InTable {
        part,
        table,
        row,
        column,
    }) = in_table(reader.docx, address)?
    else {
        return Ok(None);
    };
    let table_id = part.ids.tables().get(table).map(|t| t.id.clone()).unwrap_or_default();
    let all = cut(reader, part, Some(&table_id), None, Cut::Block, &table_id)?;
    let found = find_table(&all, &table_id).cloned();
    let Some(mut table) = found else {
        return Err(AddressError::not_found(
            format!("{} is not in its story in this view", repr_str(address)),
            Vec::new(),
        ));
    };
    match column {
        None => {
            table.rows.retain(|line| line.first().is_some_and(|c| c.row == row));
            table.header_rows = 0;
            Ok(Some(records(None, vec![Record::Table(Box::new(table))])))
        }
        Some(column) => {
            let cell = table.rows.iter().flatten().find(|c| {
                c.row <= row && row < c.row + c.row_span && c.column <= column && column < c.column + c.column_span
            });
            match cell {
                Some(cell) => Ok(Some(Selected::Records {
                    story: None,
                    records: cell.records.clone(),
                    cell: true,
                })),
                None => Err(AddressError::not_found(
                    format!("{} is not in its story in this view", repr_str(address)),
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
    let wanted = address::section(reader.docx, range)?;
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
    Err(AddressError::not_found(
        format!("no section {}", repr_str(range)),
        Vec::new(),
    ))
}
