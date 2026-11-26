//! Owns: paging a reply as today's tools page it (`page_text`, `page_list` in ooxml-edit's
//! `results.py`): the cursor `c<offset>`, a text page breaking at a line end in its second
//! half, a list page of whole items; and the refusal of a cursor no page gave, whose way
//! out is the first page.
//!
//! Must not: know what is paged.

use crate::{Refusal, ToolError, TryCall, WayOut};
use ooxml_base::pyfmt::repr_str;

/// The offset a cursor names (`None`: the start).
fn offset(cursor: Option<&str>, first: &TryCall) -> Result<usize, ToolError> {
    match cursor.filter(|c| !c.is_empty()) {
        None => Ok(0),
        Some(c) => c
            .strip_prefix('c')
            .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|d| d.parse::<usize>().ok())
            .ok_or_else(|| bad_cursor(format!("cursor {} is not one this tool gave", repr_str(c)), first)),
    }
}

fn past_end(cursor: Option<&str>, first: &TryCall) -> ToolError {
    bad_cursor(
        format!("cursor {} is past the end", repr_str(cursor.unwrap_or(""))),
        first,
    )
}

/// A cursor refused: the way out is `first`, the call for the first page.
fn bad_cursor(message: String, first: &TryCall) -> ToolError {
    ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(first.clone())).at("cursor")
}

/// One page of `text` from `cursor`, at most `limit` characters, and the next page's
/// cursor: a page breaks at a line end in its second half where there is one.
///
/// # Errors
/// `invalid_arguments` for a cursor no page gave, or one past the end.
pub fn page_text(
    text: &str,
    cursor: Option<&str>,
    limit: usize,
    first: TryCall,
) -> Result<(String, Option<String>), ToolError> {
    let chars: Vec<char> = text.chars().collect();
    let start = offset(cursor, &first)?;
    if start > chars.len() {
        return Err(past_end(cursor, &first));
    }
    let end = start + limit;
    if end >= chars.len() {
        return Ok((chars.get(start..).unwrap_or(&[]).iter().collect(), None));
    }
    let from = start + limit / 2;
    let end = chars
        .get(from..end)
        .and_then(|window| window.iter().rposition(|c| *c == '\n'))
        .map_or(end, |k| from + k + 1);
    Ok((
        chars.get(start..end).unwrap_or(&[]).iter().collect(),
        Some(format!("c{end}")),
    ))
}

/// One page of `items` from `cursor`, at most `limit` of them, the total and the next
/// page's cursor.
///
/// # Errors
/// `invalid_arguments` for a cursor no page gave, or one past the end.
pub fn page_list<T: Clone>(
    items: &[T],
    cursor: Option<&str>,
    limit: usize,
    first: TryCall,
) -> Result<(Vec<T>, usize, Option<String>), ToolError> {
    let start = offset(cursor, &first)?;
    if start > items.len() {
        return Err(past_end(cursor, &first));
    }
    let page: Vec<T> = items.iter().skip(start).take(limit).cloned().collect();
    let end = start + page.len();
    let next = (end < items.len()).then(|| format!("c{end}"));
    Ok((page, items.len(), next))
}
