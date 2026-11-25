//! Owns: [`select`] as it read a range's text before the range was read once
//! ([`crate::address::Address`]), frozen as the reference the typed one is compared with
//! (lane ph2-types6, PR 7).
//!
//! Must not: be used but by tests.

use super::*;
use crate::address::resolve::reference::expand_heading;
use crate::address::span_ends;

/// The records `range` selects.
///
/// # Errors
/// A [`RangeError`] when it names nothing or is malformed.
pub(crate) fn select(reader: &mut Reader<'_>, range: Option<&str>) -> Result<Vec<Selected>, RangeError> {
    let docx = reader.docx;
    let Some(body) = docx.body() else { return Ok(Vec::new()) };
    let Some(range) = range else {
        return Ok(vec![records(None, reader.story(body))]);
    };
    let range = expand_heading(docx, range)?;
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
