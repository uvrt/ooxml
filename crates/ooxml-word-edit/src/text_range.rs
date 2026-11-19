//! Owns: the text range an address names, as docx-agent's `Document.range` reads one
//! (`parse_range`, `_text_of`): a range id (`p:A@4:11`, `p:A@3..p:B@8`), a position
//! (`p:A@5`), or the text a paragraph, a cell, a bookmark or a hyperlink holds -- and the
//! range's paragraphs as `TextRange._entries` checks them ([`Span`]: one story, in order),
//! its text and its context (`TextRange.text`, `context`).
//!
//! Must not: change anything, or read a view other than the current one (edits take it).

use crate::set_text::{find, Found};
use crate::text::paragraph_text;
use crate::work::Work;
use crate::{EditError, Result, WordDoc};
use ooxml_base::pyfmt::repr_str;
use ooxml_word_model::address::resolve::cell_at;
use ooxml_word_model::address::{
    bookmark, bookmark_range, in_table, parse_range, resolve, span_ends, unknown, AddressError, RangeText,
};
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_word_model::ids::Entry;

const WANTED: &str = "give a range (p:A@4:11), a position (p:A@5), a paragraph, a cell or a bookmark";

/// The range `address` names (docx-agent's `parse_range`).
///
/// # Errors
/// The address's refusal, coded as docx-agent's tools code it.
pub(crate) fn range(doc: &WordDoc, docx: &Docx, address: &str) -> Result<RangeText> {
    if let Some(found) = parse_range(address) {
        find(doc, docx, &found.start_id)?;
        return Ok(found);
    }
    if span_ends(address).map_err(EditError::Address)?.is_some() {
        return Err(EditError::Address(AddressError::invalid(
            format!("{address} is a span of blocks, not text: {WANTED}, or a range across paragraphs (p:A@3..p:B@8)"),
            Vec::new(),
        )));
    }
    if address.starts_with("hl:") {
        return hyperlink_range(doc, docx, address);
    }
    if address.starts_with("bm:") || !address.contains([':', '@', '/', '#']) {
        let marked = bookmark(docx, address).map_err(EditError::Address)?;
        return bookmark_range(docx, &marked).map_err(EditError::Address);
    }
    if (address.starts_with("cc:") || address.starts_with("cc@"))
        && ooxml_word_model::docx::controls::controls(docx)
            .iter()
            .any(|c| c.id == address)
    {
        return Err(EditError::Address(AddressError::invalid(
            format!("{address} is a contentcontrol, not text: {WANTED}"),
            Vec::new(),
        )));
    }
    if let Some(cell) = cell_range(docx, address)? {
        return Ok(cell);
    }
    let followed = doc.ids().follow(address);
    let (_, entry) = resolve(docx, &followed).map_err(EditError::Address)?;
    match entry {
        Entry::Paragraph(_) => {
            let found = find(doc, docx, address)?;
            let length = paragraph_length(&found);
            Ok(RangeText {
                start_id: address.to_owned(),
                start: 0,
                end_id: address.to_owned(),
                end: length,
            })
        }
        Entry::Table(_) => Err(EditError::Address(AddressError::invalid(
            format!("{address} is a table, not text: {WANTED}"),
            Vec::new(),
        ))),
    }
}

/// A cell's text: its first paragraph's start to its last one's end; `None` for an
/// address that is no cell; a row refused.
fn cell_range(docx: &Docx, address: &str) -> Result<Option<RangeText>> {
    let Some(found) = in_table(docx, address).map_err(EditError::Address)? else {
        return Ok(None);
    };
    let Some(column) = found.column else {
        return Err(EditError::Address(AddressError::invalid(
            format!("{address} is a row, not text: {WANTED}"),
            Vec::new(),
        )));
    };
    let refuse = || EditError::Address(unknown(docx, address));
    let table = found.part.ids.tables().get(found.table).ok_or_else(refuse)?;
    let (row_path, _) = table.rows.get(found.row).ok_or_else(refuse)?;
    let row = at_path(found.part.root(), row_path).ok_or_else(refuse)?;
    let cell = cell_at(row, column).ok_or_else(refuse)?;
    let mut cell_path = row_path.to_vec();
    cell_path.push(cell);
    let paragraphs = found.part.ids.paragraphs_within(&cell_path);
    let (Some(first), Some(last)) = (paragraphs.first(), paragraphs.last()) else {
        return Err(refuse());
    };
    let work = Work::of(&last.element, Vec::new());
    let length = paragraph_text(&work, work.root()).chars().count();
    Ok(Some(RangeText {
        start_id: first.id.clone(),
        start: 0,
        end_id: last.id.clone(),
        end: length,
    }))
}

/// A hyperlink's text (`hl:<paragraph>/<n>`).
fn hyperlink_range(doc: &WordDoc, docx: &Docx, address: &str) -> Result<RangeText> {
    let not_found = || {
        EditError::Address(AddressError::not_found(
            format!("{address} is not a hyperlink id"),
            Vec::new(),
        ))
    };
    let (paragraph, index) = address
        .strip_prefix("hl:")
        .and_then(|rest| rest.rsplit_once('/'))
        .ok_or_else(not_found)?;
    let index: usize = index.parse().map_err(|_| not_found())?;
    let found = find(doc, docx, paragraph)?;
    let (start, end) = crate::links::hyperlink_span(&found.element, index).ok_or_else(|| {
        EditError::Address(AddressError::not_found(
            format!("{paragraph} has no hyperlink {index}"),
            Vec::new(),
        ))
    })?;
    Ok(RangeText {
        start_id: paragraph.to_owned(),
        start,
        end_id: paragraph.to_owned(),
        end,
    })
}

fn paragraph_length(found: &Found) -> usize {
    let work = Work::of(&found.element, Vec::new());
    paragraph_text(&work, work.root()).chars().count()
}

/// A range's paragraphs, checked as docx-agent's `TextRange._entries` checks them.
pub(crate) struct Span {
    /// The story part.
    pub(crate) part: String,
    /// Its first paragraph through its last, in order.
    pub(crate) paragraphs: Vec<Found>,
    /// Characters from its first paragraph's start.
    pub(crate) start: usize,
    /// Characters from its last paragraph's start.
    pub(crate) end: usize,
}

impl Span {
    /// The range's paragraphs.
    ///
    /// # Errors
    /// `not_found` for an end that is no paragraph, `unit` for a range across stories or
    /// one ending before it starts.
    pub(crate) fn of(doc: &WordDoc, docx: &Docx, range: &RangeText) -> Result<Span> {
        let id = range_id(range);
        let first = find(doc, docx, &range.start_id).map_err(|_| no_paragraphs(&id))?;
        let last = find(doc, docx, &range.end_id).map_err(|_| no_paragraphs(&id))?;
        if first.part != last.part {
            return Err(EditError::Address(AddressError::unit(format!(
                "{id} runs across stories"
            ))));
        }
        let part = docx
            .part(&first.part)
            .ok_or_else(|| EditError::NoPart(first.part.clone()))?;
        let index = |f: &Found| part.ids.paragraph_at(&f.path).map(|p| p.index);
        let (Some(a), Some(b)) = (index(&first), index(&last)) else {
            return Err(no_paragraphs(&id));
        };
        if b < a {
            return Err(EditError::Address(AddressError::unit(format!(
                "{id} ends before it starts"
            ))));
        }
        let mut paragraphs = Vec::with_capacity(b - a + 1);
        for k in a..=b {
            let p = part.ids.paragraphs().get(k).ok_or_else(|| no_paragraphs(&id))?;
            paragraphs.push(Found {
                part: first.part.clone(),
                path: p.path.clone(),
                id: p.id.clone(),
                state: p.state,
                element: std::sync::Arc::clone(&p.element),
            });
        }
        Ok(Span {
            part: first.part,
            paragraphs,
            start: range.start,
            end: range.end,
        })
    }

    /// Whether it is within one paragraph.
    pub(crate) fn single(&self) -> bool {
        self.paragraphs.len() == 1
    }

    /// Its first paragraph.
    pub(crate) fn first(&self) -> Option<&Found> {
        self.paragraphs.first()
    }

    /// Its last paragraph.
    pub(crate) fn last(&self) -> Option<&Found> {
        self.paragraphs.last()
    }

    /// Its text; paragraph ends are `\n` (`TextRange.text`).
    pub(crate) fn text(&self) -> String {
        let texts: Vec<Vec<char>> = self.paragraphs.iter().map(|f| text_of(f).chars().collect()).collect();
        let slice = |t: &[char], a: usize, b: usize| -> String {
            t.get(a.min(t.len())..b.min(t.len()).max(a.min(t.len())))
                .unwrap_or_default()
                .iter()
                .collect()
        };
        match texts.as_slice() {
            [] => String::new(),
            [one] => slice(one, self.start, self.end),
            [first, middle @ .., last] => {
                let mut out = vec![slice(first, self.start, first.len())];
                out.extend(middle.iter().map(|t| t.iter().collect::<String>()));
                out.push(slice(last, 0, self.end));
                out.join("\n")
            }
        }
    }

    /// Its text with up to `width` characters around it (`TextRange.context`).
    pub(crate) fn context(&self, width: usize) -> String {
        let (Some(first), Some(last)) = (self.first(), self.last()) else {
            return String::new();
        };
        let first: Vec<char> = text_of(first).chars().collect();
        let last: Vec<char> = text_of(last).chars().collect();
        let before: String = first
            .get(self.start.saturating_sub(width).min(first.len())..self.start.min(first.len()))
            .unwrap_or_default()
            .iter()
            .collect();
        let after: String = last
            .get(self.end.min(last.len())..(self.end + width).min(last.len()))
            .unwrap_or_default()
            .iter()
            .collect();
        format!("{before}[{}]{after}", self.text())
    }
}

fn text_of(found: &Found) -> String {
    let work = Work::of(&found.element, Vec::new());
    paragraph_text(&work, work.root())
}

fn no_paragraphs(id: &str) -> EditError {
    EditError::Address(AddressError::not_found(
        format!("{id} does not run between paragraphs"),
        Vec::new(),
    ))
}

/// A range's id as docx-agent writes it (`TextRange.id`).
pub(crate) fn range_id(range: &RangeText) -> String {
    if range.start_id == range.end_id {
        return format!("{}@{}:{}", range.start_id, range.start, range.end);
    }
    format!("{}@{}..{}@{}", range.start_id, range.start, range.end_id, range.end)
}

/// The one place `text` occurs in the document's current text, as a range id (docx-agent's
/// `Document.anchor`: every story's paragraphs, case-sensitive, not across paragraphs).
///
/// # Errors
/// `not_found` when it occurs nowhere; `ambiguous`, listing the places with their
/// context, when it occurs more than once.
pub fn anchor(doc: &WordDoc, text: &str) -> Result<String> {
    let docx = doc.docx()?;
    let pattern = crate::find::Literal::new(text, true, false, "")?;
    let found = crate::find::matches(&docx, &pattern, false)?;
    let ranges: Vec<RangeText> = found
        .into_iter()
        .map(|(start_id, start, end_id, end, _)| RangeText {
            start_id,
            start,
            end_id,
            end,
        })
        .collect();
    let wanted = repr_str(text);
    match ranges.as_slice() {
        [] => Err(EditError::Address(AddressError::not_found(
            format!("{wanted} occurs nowhere"),
            Vec::new(),
        ))),
        [one] => Ok(range_id(one)),
        many => {
            let described: Vec<String> = many
                .iter()
                .map(|r| {
                    let context = Span::of(doc, &docx, r).map(|s| s.context(20)).unwrap_or_default();
                    format!("{} {}", range_id(r), repr_str(&context))
                })
                .collect();
            let listed = described.iter().take(10).cloned().collect::<Vec<_>>().join("; ");
            let more = if many.len() > 10 {
                format!(" and {} more", many.len() - 10)
            } else {
                String::new()
            };
            let message = format!(
                "{} places match {wanted}: {listed}{more}; give occurrence= or within= to choose one",
                many.len()
            );
            Err(EditError::Address(AddressError {
                code: "ambiguous",
                message,
                options: described.into_iter().take(50).collect(),
            }))
        }
    }
}
