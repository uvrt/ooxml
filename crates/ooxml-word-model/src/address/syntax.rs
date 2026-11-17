//! Owns: how an address is written (docx-agent's `edit/addresses.py` and `edit/ranges.py`
//! patterns): a span of blocks (`p:A..t:B`, either end open), a text position or range
//! (`p:A@5`, `p:A@4:11`, `p:A@3..p:B@8`), a cell or row below a table (`t:X/c1,2`,
//! `t:X/r1`), and the refusals of a malformed one, worded as docx-agent words them.
//!
//! Must not: look at a document; whether an address names anything is [`mod@super::resolve`]'s.

use super::AddressError;
use ooxml_base::pyfmt::repr_str;

/// How a span is written, in every refusal of one.
pub const SPAN_SYNTAX: &str = "a span is two block ids, <first>..<last> (p:A..t:B); p:A.. runs to the end of p:A's container (the body, a cell), ..t:B from its start";

/// A span's two ends, `None` for an open one.
pub type Ends<'a> = (Option<&'a str>, Option<&'a str>);

/// Whether `text` is all ASCII digits and not empty (`\d+`).
fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// A text position or range end: `<paragraph>@<offset>` or `<paragraph>@<start>:<end>`
/// (`.+@\d+(?::\d+)?`, matched whole).
#[must_use]
pub fn is_position(end: &str) -> bool {
    let Some((left, right)) = end.rsplit_once('@') else {
        return false;
    };
    if left.is_empty() {
        return false;
    }
    match right.split_once(':') {
        Some((a, b)) => digits(a) && digits(b),
        None => digits(right),
    }
}

/// The two ends of a span of blocks (`None` for an open one), or `None` when `address` is
/// not a span of blocks -- one id, or a text range across paragraphs (`p:A@3..p:B@8`).
///
/// # Errors
/// A malformed span, with the syntax.
pub fn span_ends(address: &str) -> Result<Option<Ends<'_>>, AddressError> {
    let Some((first, last)) = address.split_once("..") else {
        return Ok(None);
    };
    let (first, last) = (first.trim(), last.trim());
    if address.contains("...") || last.contains("..") || (first.is_empty() && last.is_empty()) {
        return Err(AddressError::invalid(
            format!("{} is not a span: {SPAN_SYNTAX}", repr_str(address)),
            Vec::new(),
        ));
    }
    let positions = [first, last].map(|end| !end.is_empty() && is_position(end));
    if positions.iter().any(|p| *p) {
        if positions.iter().all(|p| *p) {
            return Ok(None);
        }
        return Err(AddressError::invalid(
            format!(
                "{} mixes a text position and a block: a range across paragraphs is p:A@3..p:B@8, offsets at both ends; {SPAN_SYNTAX}",
                repr_str(address)
            ),
            Vec::new(),
        ));
    }
    Ok(Some((non_empty(first), non_empty(last))))
}

fn non_empty(text: &str) -> Option<&str> {
    (!text.is_empty()).then_some(text)
}

/// Whether `address` is a text range or position (`p:A@4:11`, `p:A@5`, `p:A@3..p:B@8`).
#[must_use]
pub fn is_text_range(address: &str) -> bool {
    if !address.contains('@') {
        return false;
    }
    if address.contains("..") {
        return matches!(span_ends(address), Ok(None));
    }
    is_position(address)
}

/// What is below a table: a row by its place, or a cell by grid row and column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inside {
    /// `t:X/r<k>`.
    Row(usize),
    /// `t:X/c<row>,<column>`.
    Cell(usize, usize),
}

/// The table and what below it a cell's or row's id names (`t:X/c1,2`, `t:X/r1`), or
/// `None` for any other form (a `tr:` row is found by its id).
#[must_use]
pub fn inside_table(address: &str) -> Option<(&str, Inside)> {
    let (table, step) = address.rsplit_once('/')?;
    if table.is_empty() {
        return None;
    }
    let inside = if let Some(cell) = step.strip_prefix('c') {
        let (r, c) = cell.split_once(',')?;
        if !(digits(r) && digits(c)) {
            return None;
        }
        Inside::Cell(r.parse().ok()?, c.parse().ok()?)
    } else {
        let r = step.strip_prefix('r').filter(|r| digits(r))?;
        Inside::Row(r.parse().ok()?)
    };
    let names_table = table.starts_with("t:") || table.starts_with("t@") || table.contains("/t:");
    names_table.then_some((table, inside))
}

/// A text range as written: its first paragraph and offset, its last paragraph and offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeText {
    /// The paragraph it starts in, as written.
    pub start_id: String,
    /// Its offset there.
    pub start: usize,
    /// The paragraph it ends in.
    pub end_id: String,
    /// Its offset there.
    pub end: usize,
}

/// `<p>@<a>:<b>`, `<p>@<a>..<q>@<c>` or the position `<p>@<a>`, as docx-agent's
/// `parse_range` reads them; `None` for any other form.
#[must_use]
pub fn parse_range(address: &str) -> Option<RangeText> {
    for (at, _) in address.match_indices('@') {
        let start_id = address.get(..at)?;
        let rest = address.get(at + 1..)?;
        if start_id.is_empty() {
            continue;
        }
        let lead = rest.bytes().take_while(u8::is_ascii_digit).count();
        let (a, tail) = rest.split_at(lead);
        if a.is_empty() {
            continue;
        }
        let a: usize = a.parse().ok()?;
        if let Some(b) = tail.strip_prefix(':').filter(|b| digits(b)) {
            let b = b.parse().ok()?;
            return Some(range(start_id, a, start_id, b));
        }
        if let Some(rest) = tail.strip_prefix("..") {
            if let Some((end_id, c)) = rest.rsplit_once('@') {
                if !end_id.is_empty() && digits(c) {
                    return Some(range(start_id, a, end_id, c.parse().ok()?));
                }
            }
        }
    }
    let (start_id, offset) = address.rsplit_once('@')?;
    if start_id.is_empty() || !digits(offset) {
        return None;
    }
    let offset = offset.parse().ok()?;
    Some(range(start_id, offset, start_id, offset))
}

fn range(start_id: &str, start: usize, end_id: &str, end: usize) -> RangeText {
    RangeText {
        start_id: start_id.to_owned(),
        start,
        end_id: end_id.to_owned(),
        end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_and_positions_read_as_docx_agent_reads_them() {
        assert_eq!(span_ends("p:A..t:B").unwrap(), Some((Some("p:A"), Some("t:B"))));
        assert_eq!(span_ends("p:A..").unwrap(), Some((Some("p:A"), None)));
        assert_eq!(span_ends("p:A@3..p:B@8").unwrap(), None);
        assert!(span_ends("p:A...p:B").is_err());
        assert!(span_ends("..").is_err());
        assert!(span_ends("p:A@3..p:B").is_err());
        assert!(is_text_range("p:A@4:11"));
        assert!(is_text_range("p:A@5"));
        assert!(!is_text_range("p@body/7"));
        assert_eq!(inside_table("t:X/c1,2"), Some(("t:X", Inside::Cell(1, 2))));
        assert_eq!(inside_table("t:X/r3"), Some(("t:X", Inside::Row(3))));
        assert_eq!(inside_table("p:X/r3"), None);
        let r = parse_range("p:A@3..p:B@8").unwrap();
        assert_eq!(
            (r.start_id.as_str(), r.start, r.end_id.as_str(), r.end),
            ("p:A", 3, "p:B", 8)
        );
        let r = parse_range("p:A@4:11").unwrap();
        assert_eq!((r.start, r.end), (4, 11));
        let r = parse_range("p:A@5").unwrap();
        assert_eq!((r.start, r.end), (5, 5));
        assert!(parse_range("p:A").is_none());
    }
}
