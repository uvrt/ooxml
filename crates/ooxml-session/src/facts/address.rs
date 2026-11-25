//! Owns: a reference's address as text (ooxml-edit's `textfacts`, ported exactly): its
//! document handle ([`qualified`]), its number format or date pattern after the address
//! ([`split`], [`date_suffix`], [`strip_format`]), a suffix that looks like one and is
//! neither ([`bad_format`]) and its refusal ([`format_error`]), its segments split at `/`
//! outside quotes ([`segments`]) and a key quoted as a reference writes it ([`quote`]).
//!
//! Python's patterns are written out by hand: `\s` is Python's white space
//! ([`is_py_space`]), `$` also matches before one final newline.
//!
//! Must not: resolve an address or read a value.

use ooxml_base::pyfmt::repr_str;
use ooxml_tools::sources::{pattern_problem, TOKENS, TOKENS_TEXT};
use ooxml_tools::{Refusal, ToolError, WayOut};

/// The number formats a reference takes, as an error lists them.
pub const FORMS: [&str; 7] = ["0", "0.0", "0.00", "#,##0", "#,##0.0", "0%", "0.0%"];

/// Python's `str.isspace` for one character (`\s` on `str`).
#[must_use]
pub fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.strip()`.
#[must_use]
pub fn py_strip(text: &str) -> &str {
    text.trim_matches(is_py_space)
}

/// Whether `text` is `d` and digits (`re.fullmatch(r"d[0-9]+", text)`).
#[must_use]
pub fn is_handle(text: &str) -> bool {
    text.strip_prefix('d')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// `d2/t:x` as `("d2", "t:x")` (`QUALIFIED`, `^(d[0-9]+)/(.+)$`); `None` without a handle.
#[must_use]
pub fn qualified(text: &str) -> Option<(&str, &str)> {
    let (handle, rest) = text.split_once('/')?;
    if !is_handle(handle) {
        return None;
    }
    // `.+$`: no newline, but one final one after it
    let kept = rest.strip_suffix('\n').unwrap_or(rest);
    (!kept.is_empty() && !kept.contains('\n')).then_some((handle, kept))
}

/// The address a reference names after its handle, where it has one.
#[must_use]
pub fn unqualified(text: &str) -> &str {
    qualified(text).map_or(text, |(_, rest)| rest)
}

/// A number format found after an address (`FORMAT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberFormat {
    /// Where its `:` is, in bytes.
    pub at: usize,
    /// The format (`#,##0.0%`).
    pub format: String,
    /// How many decimals.
    pub decimals: usize,
    /// Whether it ends in `%`.
    pub percent: bool,
}

/// `text` read as a format from its start, then only white space (`FORMAT` after its `:`):
/// the format, its decimals and percent.
fn format_from(text: &str) -> Option<(String, usize, bool)> {
    let rest = text.strip_prefix("#,##").unwrap_or(text);
    let grouped = rest.len() != text.len();
    let rest = rest.strip_prefix('0')?;
    let zeros = |r: &str| r.bytes().take_while(|b| *b == b'0').count();
    let (decimals, rest) = match rest.strip_prefix('.') {
        Some(after) if (1..=9).contains(&zeros(after)) => {
            let n = zeros(after);
            (n, after.get(n..).unwrap_or(""))
        }
        _ => (0, rest),
    };
    let (percent, rest) = rest.strip_prefix('%').map_or((false, rest), |r| (true, r));
    if !rest.chars().all(is_py_space) {
        return None;
    }
    let mut format = String::from(if grouped { "#,##0" } else { "0" });
    if decimals > 0 {
        format.push('.');
        format.push_str(&"0".repeat(decimals));
    }
    if percent {
        format.push('%');
    }
    Some((format, decimals, percent))
}

/// The first number format in `address` (`FORMAT.search`).
#[must_use]
pub fn format_at(address: &str) -> Option<NumberFormat> {
    address.match_indices(':').find_map(|(at, _)| {
        let (format, decimals, percent) = format_from(address.get(at + 1..)?)?;
        Some(NumberFormat {
            at,
            format,
            decimals,
            percent,
        })
    })
}

/// Whether `format` is one a reference takes (`FORMAT.fullmatch(":" + format)`).
#[must_use]
pub fn is_number_format(format: &str) -> bool {
    format_from(format).is_some()
}

/// The number format `format` is: its decimals and percent.
#[must_use]
pub fn number_format(format: &str) -> Option<(usize, bool)> {
    format_from(format).map(|(_, decimals, percent)| (decimals, percent))
}

/// Where the first `:` after the source's prefix is, outside quotes -- the start of a date
/// pattern's suffix -- in `address` (bytes); `None` without one (`_suffix_at`).
fn suffix_at(address: &str) -> Option<usize> {
    let from = qualified(address).map_or(0, |(handle, _)| handle.len() + 1);
    let colon = from + address.get(from..)?.find(':')?;
    let mut quoted = false;
    for (at, c) in address.get(colon + 1..)?.char_indices() {
        if c == '\'' {
            quoted = !quoted;
        } else if c == ':' && !quoted {
            return Some(colon + 1 + at);
        }
    }
    None
}

/// `(address without it, pattern)` when `address` ends in a date pattern; `None` otherwise
/// -- a number format, a column's own colon (`date_suffix`).
#[must_use]
pub fn date_suffix(address: &str) -> Option<(&str, &str)> {
    let at = suffix_at(address)?;
    let pattern = py_strip(address.get(at + 1..)?);
    if is_number_format(pattern) || pattern_problem(pattern).is_some() {
        return None;
    }
    Some((address.get(..at)?, pattern))
}

/// Whether a reference's format ([`split`]'s) is a date pattern, not a number format.
#[must_use]
pub fn is_date_pattern(format: Option<&str>) -> bool {
    format.is_some_and(|f| !is_number_format(f))
}

/// `address` without its number format or date pattern (`strip_format`).
#[must_use]
pub fn strip_format(address: &str) -> &str {
    if let Some((body, _)) = date_suffix(address) {
        return body;
    }
    match format_at(address) {
        Some(found) => address.get(..found.at).unwrap_or(address),
        None => address,
    }
}

/// One segment of an address: its text, unquoted, and whether it was quoted.
pub type Segment = (String, bool);

/// `(source, segments, format)` of a reference's address: the source with its document
/// handle (`d2/t:x`), the segments after it (row, column), the number format or date
/// pattern ([`is_date_pattern`]) (`split`).
#[must_use]
pub fn split(address: &str) -> (String, Vec<Segment>, Option<String>) {
    let (body, format) = match date_suffix(address) {
        Some((body, pattern)) => (body, Some(pattern.to_owned())),
        None => match format_at(address) {
            Some(found) => (address.get(..found.at).unwrap_or(address), Some(found.format)),
            None => (address, None),
        },
    };
    let mut parts = segments(py_strip(body));
    if parts.len() > 1 && parts.first().is_some_and(|(first, quoted)| !quoted && is_handle(first)) {
        let (handle, _) = parts.remove(0);
        if let Some(first) = parts.first_mut() {
            first.0 = format!("{handle}/{}", first.0);
            first.1 = false;
        }
    }
    let mut parts = parts.into_iter();
    let (source, _) = parts.next().unwrap_or_default();
    (source, parts.collect(), format)
}

/// `body` split at `/` outside quotes: each segment and whether it was quoted (`_segments`).
#[must_use]
pub fn segments(body: &str) -> Vec<Segment> {
    let chars: Vec<char> = body.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if c == '\'' && current.is_empty() {
            let mut end = i + 1;
            while let Some(&x) = chars.get(end) {
                if x == '\'' {
                    if chars.get(end + 1) == Some(&'\'') {
                        current.push('\'');
                        end += 2;
                        continue;
                    }
                    break;
                }
                current.push(x);
                end += 1;
            }
            quoted = true;
            i = end + 1;
            continue;
        }
        if c == '/' {
            out.push((std::mem::take(&mut current), quoted));
            quoted = false;
        } else {
            current.push(c);
        }
        i += 1;
    }
    out.push((current, quoted));
    out
}

/// A key or column as a reference writes it: quoted when it holds `/` or `'`, is `*`, or
/// starts with `#` or `{` (`quote`).
#[must_use]
pub fn quote(part: &str) -> String {
    if part.contains(['/', '\'']) || part == "*" || part.starts_with(['#', '{']) {
        format!("'{}'", part.replace('\'', "''"))
    } else {
        part.to_owned()
    }
}

/// Whether a suffix looks like a number format (`ATTEMPTED_FORMAT`): digits, `#`, `.`,
/// `,`, `%` and spaces with a digit or `#`, or a Python-style `.2f`.
fn attempted_format(suffix: &str) -> bool {
    let numberish = |c: char| c.is_ascii_digit() || matches!(c, '#' | '.' | ',' | '%') || is_py_space(c);
    if suffix.chars().all(numberish) && suffix.chars().any(|c| c.is_ascii_digit() || c == '#') {
        return true;
    }
    let core = py_strip(suffix);
    let core = core.strip_prefix('.').unwrap_or(core);
    let mut chars = core.chars();
    let Some(last) = chars.next_back() else {
        return false;
    };
    matches!(last, 'f' | 'F' | 'g' | 'G' | 'd' | 'D' | '%') && chars.all(|c| c.is_ascii_digit())
}

/// Whether a suffix looks like a date pattern (`ATTEMPTED_DATE`): the date letters and
/// separators, with a `d`, `m` or `y`.
fn attempted_date(suffix: &str) -> bool {
    let dmy = |c: char| matches!(c, 'd' | 'D' | 'm' | 'M' | 'y' | 'Y');
    let allowed = |c: char| {
        dmy(c)
            || matches!(
                c,
                'h' | 'H' | 's' | 'S' | 'T' | 't' | '%' | '/' | '-' | '.' | ',' | ':' | '\'' | '’' | '"' | '\\'
            )
            || is_py_space(c)
    };
    suffix.chars().any(dmy) && suffix.chars().all(allowed)
}

/// The suffix of `address` when it looks like a number format or a date pattern and is
/// neither (`bad_format`): after the last `:` of the last segment, one that looks like a
/// number format; else after the first `:` past the source's prefix, one that looks like a
/// date pattern. `None` otherwise, or when that segment is quoted.
#[must_use]
pub fn bad_format(address: &str) -> Option<String> {
    if format_at(address).is_some() || date_suffix(address).is_some() {
        return None;
    }
    let stripped = py_strip(address);
    let parts = segments(stripped);
    let source = parts.len() == 1 || (parts.len() == 2 && parts.first().is_some_and(|(first, _)| is_handle(first)));
    if let Some((last, quoted)) = parts.last() {
        let colons = last.matches(':').count();
        if !quoted && colons >= if source { 2 } else { 1 } {
            let suffix = last.rsplit_once(':').map_or("", |(_, s)| s);
            if !py_strip(suffix).is_empty() && attempted_format(suffix) {
                return Some(suffix.to_owned());
            }
        }
    }
    let attempt = suffix_at(stripped).and_then(|at| stripped.get(at + 1..)).unwrap_or("");
    (!py_strip(attempt).is_empty() && attempted_date(attempt)).then(|| attempt.to_owned())
}

/// The refusal of a reference whose suffix [`bad_format`] found: what it is not, the forms
/// there are -- number formats and date tokens -- and the reference written with the
/// nearest one (`format_error`).
#[must_use]
pub fn format_error(address: &str, suffix: &str, field: Option<&str>) -> ToolError {
    let body = py_strip(address);
    let error = if attempted_format(suffix) {
        number_format_error(address, body, suffix)
    } else {
        let body = suffix_at(body).and_then(|at| body.get(..at)).unwrap_or(body);
        let why = pattern_problem(py_strip(suffix))
            .map(|message| {
                message
                    .split_once(": ")
                    .map_or(message.clone(), |(_, why)| why.to_owned())
            })
            .map_or(String::new(), |why| format!(": {why}"));
        ToolError::refusal(
            Refusal::InvalidArguments,
            format!(
                "{{{{{address}}}}}: {} is not a date pattern a reference takes{why}. A date pattern is the date \
                 letters of a number format -- {TOKENS_TEXT} -- with anything else written as given; e.g. \
                 {{{{{body}:d mmmm yyyy}}}} or {{{{{body}:dd/mm/yyyy}}}}",
                repr_str(py_strip(suffix))
            ),
            way_out(&TOKENS),
        )
    };
    match field {
        Some(field) => error.at(field),
        None => error,
    }
}

/// The refusal of a suffix that looks like a number format and is none.
fn number_format_error(address: &str, body: &str, suffix: &str) -> ToolError {
    let body = body.rfind(':').and_then(|at| body.get(..at)).unwrap_or(body);
    let grouped = suffix.contains("#,");
    let point = if suffix.contains('.') || grouped { '.' } else { ',' };
    // the first point followed by digits or `#`: that many decimals, at most 9
    let mut count = suffix
        .match_indices(point)
        .find_map(|(at, _)| {
            let run = suffix
                .get(at + 1..)?
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '#')
                .count();
            (run > 0).then_some(run.min(9))
        })
        .unwrap_or(0);
    if let Some(digit) = python_style(suffix) {
        count = digit;
    }
    let mut near = String::from(if grouped { "#,##0" } else { "0" });
    if count > 0 {
        near.push('.');
        near.push_str(&"0".repeat(count));
    }
    if suffix.contains('%') {
        near.push('%');
    }
    ToolError::refusal(
        Refusal::InvalidArguments,
        format!(
            "{{{{{address}}}}}: {} is not a number format a reference takes. Write :0 (a whole number), :0.0 to \
             :0.000000000 (that many decimals, one 0 each), :#,##0 or :#,##0.0 (thousands grouped), with % at \
             the end for a percent (:0.0%); a date takes a date pattern of {} (:d mmmm yyyy, :dd/mm/yyyy); e.g. \
             {{{{{body}:{near}}}}}",
            repr_str(py_strip(suffix)),
            TOKENS.join(" ")
        ),
        way_out(&FORMS),
    )
}

/// The decimals of a Python-style format (`\s*\.([0-9])[fF%]\s*`: `.2f`).
fn python_style(suffix: &str) -> Option<usize> {
    let core = py_strip(suffix).strip_prefix('.')?;
    let mut chars = core.chars();
    let digit = chars.next()?.to_digit(10)?;
    let kind = chars.next()?;
    (matches!(kind, 'f' | 'F' | '%') && chars.next().is_none()).then(|| usize::try_from(digit).unwrap_or(0))
}

/// A way out naming `options`.
pub(crate) fn way_out(options: &[&str]) -> WayOut {
    WayOut::names(options.iter().copied()).unwrap_or_else(|| WayOut::one(serde_json::Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(parts: &[(&str, bool)]) -> Vec<Segment> {
        parts.iter().map(|(t, q)| ((*t).to_owned(), *q)).collect()
    }

    /// `test_a_date_suffix_is_read_only_after_the_sources_prefix`.
    #[test]
    fn a_date_suffix_is_read_only_after_the_sources_prefix() {
        assert_eq!(
            date_suffix("d:x/#1/when:dd/mm/yyyy"),
            Some(("d:x/#1/when", "dd/mm/yyyy"))
        );
        assert_eq!(
            date_suffix("d2/t:c/N-1/when:d mmmm"),
            Some(("d2/t:c/N-1/when", "d mmmm"))
        );
        assert_eq!(date_suffix("d:dmy"), None);
        assert_eq!(date_suffix("d:x/#1/avg:0.0"), None);
        assert_eq!(date_suffix("d:x/'a:dd'/v"), None);
        assert_eq!(date_suffix("d:x/#1/avg:total"), None);
        assert_eq!(strip_format("d:x/#1/when:d mmmm yyyy"), "d:x/#1/when");
        assert_eq!(
            split("d:x/#1/when:dd/mm/yyyy"),
            (
                "d:x".to_owned(),
                seg(&[("#1", false), ("when", false)]),
                Some("dd/mm/yyyy".to_owned())
            )
        );
    }

    #[test]
    fn an_address_splits_into_its_source_segments_and_format() {
        assert_eq!(
            split("d2/t:checks/'A/7'/rating:#,##0.0"),
            (
                "d2/t:checks".to_owned(),
                seg(&[("A/7", true), ("rating", false)]),
                Some("#,##0.0".to_owned())
            )
        );
        assert_eq!(split("d:total"), ("d:total".to_owned(), Vec::new(), None));
        assert_eq!(segments("'it''s'/x"), seg(&[("it's", true), ("x", false)]));
        assert_eq!(quote("A/7"), "'A/7'");
        assert_eq!(quote("it's"), "'it''s'");
        assert_eq!(quote("#1"), "'#1'");
        assert_eq!(quote("N-1"), "N-1");
    }

    #[test]
    fn a_number_format_is_found_at_the_end_only() {
        let found = format_at("d:x/#1/avg:0.0%").unwrap();
        assert_eq!(
            (found.format.as_str(), found.decimals, found.percent),
            ("0.0%", 1, true)
        );
        assert_eq!(format_at("d:x/#1/avg:#,##0 ").unwrap().format, "#,##0");
        for address in ["d:x/#1/avg:0.20", "d:x/#1/avg:0.0000000000", "d:x/#1/avg:0,0", "d:2024"] {
            assert_eq!(format_at(address), None, "{address}");
        }
        assert_eq!(qualified("d2/t:x"), Some(("d2", "t:x")));
        assert_eq!(qualified("d:x/y"), None);
    }

    /// Part of `test_a_quoted_column_or_a_plain_unknown_one_is_not_read_as_a_format`.
    #[test]
    fn a_bad_suffix_is_one_that_looks_like_a_format() {
        assert_eq!(bad_format("d:2024"), None);
        assert_eq!(bad_format("d:x/#1/avg:0.0"), None);
        assert_eq!(bad_format("d:x/#1/avg:0.20").as_deref(), Some("0.20"));
        assert_eq!(bad_format("d:x/#1/avg:.2f").as_deref(), Some(".2f"));
        assert_eq!(bad_format("d:x/#1/'avg:0.20'"), None);
        assert_eq!(bad_format("d:x/#1/avg:total"), None);
        assert_eq!(
            bad_format("d:x/#1/when:DD/MM/YYYY HH:mm").as_deref(),
            Some("DD/MM/YYYY HH:mm")
        );
        assert_eq!(bad_format("d:x/#1/when:%d-%m-%Y").as_deref(), Some("%d-%m-%Y"));
    }
}
