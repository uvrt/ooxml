//! Owns: what an address names in a version (docx-agent's `_direct`, `_resolve`, `_unknown`,
//! `get`, `bookmark`, `_control`, `_section_record` and `_scope_paragraphs`): a block by id,
//! a row or cell, a bookmark's or text range's paragraphs and offsets, a content control, a
//! section's paragraphs, a heading's range as the span it is, and the paragraphs any
//! address holds -- each refusal worded as docx-agent words it, with the ids that would do.
//!
//! Renamed ids are not followed: the Rust session keeps no aliases yet (they come with the
//! ledger in the document, phase P2), so an id is found as it stands.
//!
//! Must not: read Markdown records, or change the document.

pub use super::marks::{bookmark, bookmark_range, marker_range, range_text, Bookmark};
use super::parsed::{Address, Form, Shape};
use super::syntax::{inside_table, is_text_range, parse_range, span_ends, Inside, RangeText, SPAN_SYNTAX};
use super::AddressError;
use crate::docx::controls::Control;
use crate::docx::outline::{body_blocks, headings, Heading};
use crate::docx::sections::{section_id, sections};
use crate::docx::text::{paragraph_text, View};
use crate::docx::{style_name, Docx, StoryPart};
use crate::ids::{Entry, W};
use ooxml_base::pyfmt::repr_str;
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// How a heading's range is written, in every refusal of one.
pub const HEADING_SYNTAX: &str = "h:<heading> is the heading and every block under it, to the next heading of its level or higher (h:p:A; h:p:A..h:p:B to the end of p:B's)";

/// The prefix of a heading's range.
pub const HEADING: &str = "h:";

/// A block an id names: its story part and its entry there.
pub type Block<'d> = (&'d StoryPart, Entry);

/// A paragraph's place in its part's index.
pub type Paragraphs<'d> = (&'d StoryPart, Vec<usize>);

/// The block `id` names as written (docx-agent's `_direct`): the story its form says, then
/// the id there.
#[must_use]
pub fn direct<'d>(docx: &'d Docx, id: &str) -> Option<Block<'d>> {
    let story = if let Some(rest) = id.strip_prefix("p@").or_else(|| id.strip_prefix("t@")) {
        rest.rsplit_once('/').map_or("", |(s, _)| s)
    } else if let Some((s, _)) = id.split_once('/') {
        s
    } else {
        "body"
    };
    let part = docx.story(story)?;
    Some((part, part.ids.get(id)?))
}

/// The block `id` names, or the refusal saying what it is instead.
///
/// # Errors
/// [`unknown`]'s.
pub fn resolve<'d>(docx: &'d Docx, id: &str) -> Result<Block<'d>, AddressError> {
    direct(docx, id).ok_or_else(|| unknown(docx, id))
}

/// The paragraph `id` names: its part and its index there.
///
/// # Errors
/// As [`resolve`]; a table's id is `not_found` as docx-agent's text ranges say it.
pub fn paragraph<'d>(docx: &'d Docx, id: &str) -> Result<(&'d StoryPart, usize), AddressError> {
    match resolve(docx, id)? {
        (part, Entry::Paragraph(k)) => Ok((part, k)),
        (_, Entry::Table(_)) => Err(AddressError::not_found(
            format!("{id} is a table, not a paragraph"),
            Vec::new(),
        )),
    }
}

/// The refusal for an id that names no paragraph or table: what the id is when it is
/// something else (docx-agent's `_unknown`).
#[must_use]
pub fn unknown(docx: &Docx, id: &str) -> AddressError {
    if id.trim().is_empty() {
        let span = if id.is_empty() {
            format!(", or {SPAN_SYNTAX}")
        } else {
            String::new()
        };
        return AddressError::invalid(
            format!(
                "an id is empty ({}): give a block id (p:..., t:...){span}",
                repr_str(id)
            ),
            Vec::new(),
        );
    }
    match span_ends(id) {
        Err(error) => return error,
        Ok(Some((a, b))) => {
            let named = [a, b].into_iter().flatten().map(str::to_owned).collect();
            return AddressError::invalid(format!("{id} is a span; give one block here (p:A or t:B)"), named);
        }
        Ok(None) => {}
    }
    if let Some(error) = refuse_inside_table(docx, id, "a paragraph or table") {
        return error;
    }
    if id.starts_with(HEADING) {
        return match expand_heading(docx, &Address::parse(id)) {
            Err(error) => error,
            Ok(span) => {
                let span = span.as_str();
                let ends = span.split("..").map(str::to_owned).collect();
                AddressError::invalid(format!("{id} is a heading's range ({span}); give one block here"), ends)
            }
        };
    }
    if id.starts_with("s:") || id.starts_with("s@") {
        if let Err(error) = section(docx, id) {
            return error;
        }
        return AddressError::invalid(
            format!("{id} is a section, not a paragraph or table: give a block in it"),
            Vec::new(),
        );
    }
    let holder = if is_text_range(id) {
        text_range(docx, id).ok().map(|r| (r.start_id, "a text range"))
    } else if docx.story(id).is_some() {
        return AddressError::invalid(
            format!("{id} is a story, not a paragraph or table: give a block in it"),
            Vec::new(),
        );
    } else if id.starts_with("bm:") || !id.contains([':', '@', '/', '#']) {
        bookmark(docx, id)
            .ok()
            .and_then(|b| bookmark_range(docx, &b).ok())
            .map(|r| (r.start_id, "a bookmark"))
    } else {
        None
    };
    if let Some((holder, what)) = holder {
        return AddressError::invalid(
            format!("{id} is {what}, not a paragraph or table: give the paragraph it is in, {holder}"),
            vec![holder],
        );
    }
    let options = docx
        .body()
        .map(|b| b.ids.paragraphs().iter().take(10).map(|p| p.id.clone()).collect())
        .unwrap_or_default();
    AddressError::not_found(format!("no paragraph or table {}", repr_str(id)), options)
}

/// A row or a cell of a table, found by its id.
#[derive(Debug, Clone)]
pub struct InTable<'d> {
    /// The table's part.
    pub part: &'d StoryPart,
    /// The table's place in the part's index.
    pub table: usize,
    /// The row's place among the table's rows.
    pub row: usize,
    /// The cell's grid column, for a cell.
    pub column: Option<usize>,
}

/// The row (`tr:`, `t:X/r1`) or cell (`t:X/c1,1`) `address` names; `Ok(None)` when it is
/// neither form.
///
/// # Errors
/// `not_found` for a row or table the document lacks.
pub fn in_table<'d>(docx: &'d Docx, address: &str) -> Result<Option<InTable<'d>>, AddressError> {
    if address.starts_with("tr:") || address.contains("/tr:") {
        for part in &docx.parts {
            for (t, table) in part.ids.tables().iter().enumerate() {
                if let Some(row) = table.rows.iter().position(|(_, r)| r == address) {
                    return Ok(Some(InTable {
                        part,
                        table: t,
                        row,
                        column: None,
                    }));
                }
            }
        }
        return Err(AddressError::not_found(
            format!("no row {}", repr_str(address)),
            Vec::new(),
        ));
    }
    let Some((table, inside)) = inside_table(address) else {
        return Ok(None);
    };
    let (part, entry) = resolve(docx, table)?;
    let Entry::Table(t) = entry else { return Ok(None) };
    let (row, column) = match inside {
        Inside::Row(r) => (r, None),
        Inside::Cell(r, c) => (r, Some(c)),
    };
    Ok(Some(InTable {
        part,
        table: t,
        row,
        column,
    }))
}

/// The refusal for a row's or cell's id where `wanted` (say "a block") is taken: its table
/// would do, or a cell's paragraphs; `None` for any other id, or a row or cell the table
/// lacks.
#[must_use]
pub fn refuse_inside_table(docx: &Docx, address: &str, wanted: &str) -> Option<AddressError> {
    let found = in_table(docx, address).ok()??;
    let entry = found.part.ids.tables().get(found.table)?;
    let table = entry.id.clone();
    let (row_path, _) = entry.rows.get(found.row)?;
    Some(match found.column {
        Some(column) => {
            // a cell the table lacks is no cell: the address is refused as naming nothing
            cell_at(crate::docx::at_path(found.part.root(), row_path)?, column)?;
            let paragraphs: Vec<String> = cell_paragraphs(&found)
                .into_iter()
                .filter_map(|k| found.part.ids.paragraphs().get(k).map(|p| p.id))
                .collect();
            let first: Vec<&str> = paragraphs.iter().take(3).map(String::as_str).collect();
            AddressError::invalid(
                format!(
                    "{address} is a cell, not {wanted}: give its table {table} or a paragraph in it ({})",
                    first.join(", ")
                ),
                std::iter::once(table).chain(paragraphs).collect(),
            )
        }
        None => AddressError::invalid(
            format!("{address} is a row, not {wanted}: give its table {table}"),
            vec![table],
        ),
    })
}

/// The range a text-range id names (`p:A@4:11`, `p:A@5`, `p:A@3..p:B@8`), its paragraphs
/// checked.
///
/// # Errors
/// As [`resolve`] for its paragraphs.
pub fn text_range(docx: &Docx, address: &str) -> Result<RangeText, AddressError> {
    let Some(found) = parse_range(address) else {
        return Err(AddressError::invalid(
            format!("{address} is not a text range: give a range (p:A@4:11), a position (p:A@5) or a range across paragraphs (p:A@3..p:B@8)"),
            Vec::new(),
        ));
    };
    resolve(docx, &found.start_id)?;
    Ok(found)
}

/// The content control `id` names.
///
/// # Errors
/// `refused`, as docx-agent's `EditError` says it.
pub fn control(controls: &[Control], id: &str) -> Result<Control, AddressError> {
    controls
        .iter()
        .find(|c| c.id == id)
        .cloned()
        .ok_or_else(|| AddressError::refused(format!("no content control {}", repr_str(id))))
}

/// The id of the section `address` names (`s:2`, `s:<paraId>`, `s:body`).
///
/// # Errors
/// `not_found` with the sections there are.
pub fn section(docx: &Docx, address: &str) -> Result<String, AddressError> {
    let found = sections(docx);
    let number = address
        .strip_prefix("s:")
        .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let wanted = match number.and_then(|n| n.parse::<usize>().ok()) {
        Some(n) if n >= 1 && n <= found.len() => found.get(n - 1).map(|s| s.id.clone()),
        _ => None,
    };
    if let Some(section) = found.iter().find(|s| s.id == address || Some(&s.id) == wanted.as_ref()) {
        return Ok(section.id.clone());
    }
    let options = found.iter().map(|s| s.id.clone()).collect();
    if number.is_some() {
        return Err(AddressError::not_found(
            format!(
                "no section {}: the document has {} (s:1 to s:{})",
                repr_str(address),
                found.len(),
                found.len()
            ),
            options,
        ));
    }
    Err(AddressError::not_found(
        format!("no section {}", repr_str(address)),
        options,
    ))
}

/// The body paragraphs (indexes) of the section `id`: after the paragraph ending the one
/// before, through its own.
#[must_use]
pub fn section_paragraphs(docx: &Docx, id: &str) -> Vec<usize> {
    let Some(body) = docx.body() else { return Vec::new() };
    let ends: Vec<(String, Option<Box<[u32]>>)> = sections(docx)
        .into_iter()
        .map(|s| {
            let end = body
                .ids
                .paragraphs()
                .iter()
                .find(|p| s.id != "s:body" && section_id(&p.id) == s.id)
                .map(|p| p.path);
            (s.id, end)
        })
        .collect();
    let mut start: Option<Box<[u32]>> = None;
    for (section, end) in ends {
        if section == id {
            return body
                .ids
                .paragraphs()
                .iter()
                .filter(|p| {
                    start.as_deref().is_none_or(|s| &*p.path > s) && end.as_deref().is_none_or(|e| &*p.path <= e)
                })
                .map(|p| p.index)
                .collect();
        }
        start = end;
    }
    Vec::new()
}

/// The body's headings, each with its text in the current view.
#[must_use]
pub fn body_headings(docx: &Docx) -> Vec<Heading> {
    let controls = crate::docx::controls::controls(docx);
    headings(&body_blocks(docx, &controls))
}

/// The body's first headings as ranges, each with its text (`h:p:A  Results`).
#[must_use]
pub fn offered(docx: &Docx) -> Vec<String> {
    body_headings(docx)
        .iter()
        .take(20)
        .map(|h| {
            let text: String = paragraph_text(&h.element, View::Current)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(60)
                .collect();
            format!("{HEADING}{}  {text}", h.id).trim_end().to_owned()
        })
        .collect()
}

/// The heading a heading's range (`h:p:A`, or the heading's own id) names.
///
/// # Errors
/// `invalid_arguments` for anything that is not a heading of the body, with the body's
/// headings as the options.
pub fn heading(docx: &Docx, given: &str) -> Result<Heading, AddressError> {
    let id = given.strip_prefix(HEADING).unwrap_or(given);
    let refuse = |message: String| AddressError::invalid(message, offered(docx));
    if id.trim().is_empty() {
        return Err(refuse(format!(
            "{} names no heading: {HEADING_SYNTAX}",
            repr_str(given)
        )));
    }
    if id.starts_with(HEADING) || matches!(span_ends(id), Ok(Some(_)) | Err(_)) {
        return Err(refuse(format!(
            "{} is not a heading's range: {HEADING_SYNTAX}",
            repr_str(given)
        )));
    }
    let (part, entry) = resolve(docx, id)
        .map_err(|error| refuse(format!("{}: {}; {HEADING_SYNTAX}", repr_str(given), error.message)))?;
    let body = docx.body().map(|b| b.part.as_str());
    let Entry::Paragraph(k) = entry else {
        return Err(refuse(format!("{given} is not a paragraph: {HEADING_SYNTAX}")));
    };
    let element = part.ids.paragraphs().get(k);
    let top = element.as_ref().is_some_and(|p| p.path.len() == 2);
    if Some(part.part.as_str()) != body || !top {
        return Err(refuse(format!(
            "{given} is not a block of the body itself (a heading in a table, text box, content control, header or note owns no blocks): {HEADING_SYNTAX}"
        )));
    }
    let own = element.as_ref().map(|p| p.id.clone()).unwrap_or_default();
    if let Some(found) = body_headings(docx).into_iter().find(|h| h.id == own) {
        return Ok(found);
    }
    let style = element
        .as_ref()
        .and_then(|p| crate::docx::find_path(&p.element, &["pPr", "pStyle"]))
        .and_then(|s| s.attribute(Some(W), "val"))
        .and_then(|s| docx.style(s))
        .or_else(|| docx.default_style("paragraph"))
        .and_then(|s| style_name(s).map(str::to_owned));
    let style = style.map_or_else(|| "None".to_owned(), |s| repr_str(&s));
    Err(refuse(format!(
        "{own} is not a heading (style {style}, no outline level): {HEADING_SYNTAX}"
    )))
}

/// `address` with a heading's range (`h:p:A`, or `h:` at a span's end) as the span of
/// blocks it is ([`Form::Span`]; a heading with no blocks under it, its own id); any other
/// address as it is.
///
/// # Errors
/// [`heading`]'s.
pub fn expand_heading(docx: &Docx, address: &Address) -> Result<Address, AddressError> {
    if !address.has(Shape::Heading) {
        return Ok(address.clone());
    }
    let Some((first, last)) = address.as_str().split_once("..") else {
        let h = heading(docx, address.as_str())?;
        return Ok(if h.last == h.id {
            Address::parse(&h.id)
        } else {
            Address::span(&h.id, &h.last)
        });
    };
    let (first, last) = (first.trim(), last.trim());
    let first = if first.starts_with(HEADING) {
        heading(docx, first)?.id
    } else {
        first.to_owned()
    };
    let last = if last.starts_with(HEADING) {
        heading(docx, last)?.last
    } else {
        last.to_owned()
    };
    Ok(Address::span(&first, &last))
}

/// The paragraphs (indexes in their part's index, in document order, tables' cells
/// included) of the elements at `paths` of `part`.
fn paragraphs_in(part: &StoryPart, paths: &[&[u32]]) -> Vec<usize> {
    let mut out: Vec<usize> = paths
        .iter()
        .flat_map(|path| part.ids.paragraphs_within(path))
        .map(|p| p.index)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The paragraphs a range of text runs through.
fn range_paragraphs<'d>(docx: &'d Docx, range: &RangeText) -> Result<Paragraphs<'d>, AddressError> {
    let (part, first) = paragraph(docx, &range.start_id)?;
    let (end_part, last) = paragraph(docx, &range.end_id)?;
    if end_part.part != part.part {
        return Err(AddressError::unit(format!(
            "{}@{}..{}@{} runs across stories",
            range.start_id, range.start, range.end_id, range.end
        )));
    }
    Ok((part, (first..=last).collect()))
}

/// The part and paragraphs (document order, tables' cells included) that `address` holds
/// -- any form: a story, a block or span, a row, a cell, a section, a bookmark, a text
/// range, a content control (docx-agent's `_scope_paragraphs`), tried in the order of
/// [`Form`].
///
/// # Errors
/// What names nothing, or is malformed.
pub fn scope_paragraphs<'d>(docx: &'d Docx, address: &Address) -> Result<Paragraphs<'d>, AddressError> {
    if address.has(Shape::Empty) {
        return Err(AddressError::invalid(
            format!("an address is empty: give a block id (p:..., t:...), a cell (t:X/c1,1), a section (s:1), a bookmark or a span ({SPAN_SYNTAX})"),
            Vec::new(),
        ));
    }
    let address = expand_heading(docx, address)?;
    let raw = address.as_str();
    if let Some(part) = docx.story(raw) {
        return Ok((part, (0..part.ids.paragraphs().len()).collect()));
    }
    match address.form() {
        Form::Malformed(error) => Err(error.clone()),
        Form::Span { first, last } => span_paragraphs(docx, first.as_deref(), last.as_deref()),
        Form::Text => {
            let range = text_range(docx, raw)?;
            range_paragraphs(docx, &range)
        }
        Form::Section => {
            let id = section(docx, raw)?;
            let body = docx
                .body()
                .ok_or_else(|| AddressError::not_found("no body".to_owned(), Vec::new()))?;
            Ok((body, section_paragraphs(docx, &id)))
        }
        Form::Bookmark | Form::Bare => {
            let found = bookmark(docx, raw)?;
            let range = bookmark_range(docx, &found)?;
            range_paragraphs(docx, &range)
        }
        Form::Control => {
            let controls = crate::docx::controls::controls(docx);
            let found = control(&controls, raw)?;
            let part = docx
                .part(&found.part)
                .ok_or_else(|| AddressError::not_found(format!("no part {}", found.part), Vec::new()))?;
            let inner = paragraphs_in(part, &[&found.path]);
            if !inner.is_empty() {
                return Ok((part, inner));
            }
            let holder = (1..found.path.len())
                .rev()
                .find_map(|n| found.path.get(..n).and_then(|p| part.ids.paragraph_at(p)));
            Ok((part, holder.map(|p| vec![p.index]).unwrap_or_default()))
        }
        // an expanded address is never blank or a heading's range: they read as an id
        Form::Row | Form::Inside { .. } | Form::Block | Form::Empty | Form::Heading => {
            if let Some(found) = in_table(docx, raw)? {
                return Ok((found.part, cell_paragraphs(&found)));
            }
            let (part, entry) = resolve(docx, raw)?;
            match entry {
                Entry::Paragraph(k) => Ok((part, vec![k])),
                Entry::Table(t) => {
                    let path = part.ids.tables().get(t).map(|t| t.path.clone()).unwrap_or_default();
                    Ok((part, paragraphs_in(part, &[&path])))
                }
            }
        }
    }
}

/// The paragraphs of a row or cell (a cell found by its grid place, merges counted).
fn cell_paragraphs(found: &InTable<'_>) -> Vec<usize> {
    let Some(table) = found.part.ids.tables().get(found.table) else {
        return Vec::new();
    };
    let Some((row_path, _)) = table.rows.get(found.row) else {
        return Vec::new();
    };
    let Some(column) = found.column else {
        return paragraphs_in(found.part, &[row_path]);
    };
    let Some(row) = crate::docx::at_path(found.part.root(), row_path) else {
        return Vec::new();
    };
    match cell_at(row, column) {
        Some(k) => {
            let mut path = row_path.to_vec();
            path.push(k);
            paragraphs_in(found.part, &[&path])
        }
        None => Vec::new(),
    }
}

/// The child index of the `w:tc` of `row` that covers grid `column`, placed by the table
/// grid's rules ([`crate::docx::tables::Grid`]); `None` where no cell covers it, or the cell
/// that does is inside a row-level wrapper (`w:sdt`, `w:customXml`: no child index of the
/// row's names it).
#[must_use]
pub fn cell_at(row: &Arc<Element>, column: usize) -> Option<u32> {
    use crate::docx::tables::{row_cells, Grid, RowSpec};
    let grid = Grid::build(std::slice::from_ref(&RowSpec::of(row)), 0);
    let (_, k) = grid.cell_at(0, column)?;
    match row_cells(row).get(k).map(|(_, place)| &place[..]) {
        Some(&[index]) => Some(index),
        _ => None,
    }
}

/// The paragraphs of the blocks of a span (either end open), both ends in one container.
fn span_paragraphs<'d>(
    docx: &'d Docx,
    first: Option<&str>,
    last: Option<&str>,
) -> Result<Paragraphs<'d>, AddressError> {
    let given = format!("{}..{}", first.unwrap_or(""), last.unwrap_or(""));
    let block_path = |id: &str| -> Result<(&'d StoryPart, Box<[u32]>), AddressError> {
        if id.starts_with("cc:") || id.starts_with("cc@") {
            let controls = crate::docx::controls::controls(docx);
            let found = control(&controls, id).map_err(|e| AddressError::not_found(e.message, Vec::new()))?;
            let part = docx
                .part(&found.part)
                .ok_or_else(|| AddressError::not_found(format!("no content control {}", repr_str(id)), Vec::new()))?;
            return Ok((part, found.path));
        }
        let (part, entry) = resolve(docx, id)?;
        let path = match entry {
            Entry::Paragraph(k) => part.ids.paragraphs().get(k).map(|p| p.path.clone()),
            Entry::Table(k) => part.ids.tables().get(k).map(|t| t.path.clone()),
        };
        Ok((part, path.unwrap_or_default()))
    };
    let anchor = block_path(first.or(last).unwrap_or(""))?;
    let (part, anchor_path) = anchor;
    let parent = anchor_path
        .get(..anchor_path.len().saturating_sub(1))
        .unwrap_or(&[])
        .to_vec();
    let end_path = match (first, last) {
        (Some(_), Some(last)) => {
            let (end_part, end) = block_path(last)?;
            if end_part.part != part.part || end.get(..end.len().saturating_sub(1)) != Some(parent.as_slice()) {
                return Err(AddressError::refused(format!(
                    "{}: a span is blocks of one container (the body, one cell); {SPAN_SYNTAX}",
                    repr_str(&given)
                )));
            }
            Some(end)
        }
        _ => None,
    };
    let container = crate::docx::at_path(part.root(), &parent);
    let Some(container) = container else {
        return Ok((part, Vec::new()));
    };
    let siblings: Vec<u32> = container
        .children()
        .iter()
        .enumerate()
        .filter(|(_, n)| matches!(n, Node::Element(e) if ["p", "tbl", "sdt"].iter().any(|l| e.name().is(Some(W), l))))
        .filter_map(|(k, _)| u32::try_from(k).ok())
        .collect();
    let place = |path: &[u32]| {
        path.last()
            .and_then(|k| siblings.iter().position(|s| s == k))
            .unwrap_or(0)
    };
    let (a, b) = match (first, end_path) {
        (Some(_), Some(end)) => (place(&anchor_path), place(&end)),
        (Some(_), None) => (place(&anchor_path), siblings.len().saturating_sub(1)),
        (None, _) => (0, place(&anchor_path)),
    };
    if b < a {
        return Err(AddressError::refused(format!(
            "{} ends before it starts",
            repr_str(&given)
        )));
    }
    let paths: Vec<Vec<u32>> = siblings
        .get(a..=b)
        .unwrap_or(&[])
        .iter()
        .map(|k| {
            let mut p = parent.clone();
            p.push(*k);
            p
        })
        .collect();
    let refs: Vec<&[u32]> = paths.iter().map(Vec::as_slice).collect();
    Ok((part, paragraphs_in(part, &refs)))
}

#[cfg(test)]
#[path = "reference.rs"]
pub(crate) mod reference;
