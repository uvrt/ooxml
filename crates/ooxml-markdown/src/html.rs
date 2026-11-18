//! Owns: the HTML that markdown-it recognises (`common/html_re.py`,
//! `common/html_blocks.py`): an open or close tag, a comment, a processing instruction,
//! a declaration, a CDATA section -- each matched where it starts, as its regular
//! expression matches it -- and the block-level tag names.
//!
//! Must not: parse HTML beyond recognising these.

use crate::chars::is_py_space;

/// CommonMark's block-level tag names.
pub(crate) const BLOCK_NAMES: &[&str] = &[
    "address",
    "article",
    "aside",
    "base",
    "basefont",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "iframe",
    "legend",
    "li",
    "link",
    "main",
    "menu",
    "menuitem",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
];

fn at(s: &[char], k: usize) -> Option<char> {
    s.get(k).copied()
}

fn skip_space(s: &[char], mut k: usize) -> usize {
    while at(s, k).is_some_and(is_py_space) {
        k += 1;
    }
    k
}

/// A tag name `[A-Za-z][A-Za-z0-9-]*` from `k`: the place after it.
fn tag_name(s: &[char], k: usize) -> Option<usize> {
    at(s, k).filter(char::is_ascii_alphabetic)?;
    let mut k = k + 1;
    while at(s, k).is_some_and(|c| c.is_ascii_alphanumeric() || c == '-') {
        k += 1;
    }
    Some(k)
}

/// An attribute ` name` or ` name = value` from `k` (white space first).
fn attribute(s: &[char], k: usize) -> Option<usize> {
    let start = skip_space(s, k);
    if start == k {
        return None;
    }
    at(s, start).filter(|c| c.is_ascii_alphabetic() || matches!(c, '_' | ':'))?;
    let mut name_end = start + 1;
    while at(s, name_end).is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-')) {
        name_end += 1;
    }
    let equals = skip_space(s, name_end);
    if at(s, equals) != Some('=') {
        return Some(name_end);
    }
    let value = skip_space(s, equals + 1);
    match at(s, value) {
        Some(q @ ('"' | '\'')) => {
            let close = s.get(value + 1..)?.iter().position(|c| *c == q);
            Some(close.map_or(name_end, |c| value + 1 + c + 1))
        }
        Some(_) => {
            let mut end = value;
            while at(s, end).is_some_and(|c| !matches!(c, '"' | '\'' | '=' | '<' | '>' | '`') && u32::from(c) > 0x20) {
                end += 1;
            }
            Some(if end > value { end } else { name_end })
        }
        None => Some(name_end),
    }
}

/// An open tag at `k`: the place after it.
pub(crate) fn open_tag(s: &[char], k: usize) -> Option<usize> {
    if at(s, k) != Some('<') {
        return None;
    }
    let mut pos = tag_name(s, k + 1)?;
    while let Some(next) = attribute(s, pos) {
        pos = next;
    }
    pos = skip_space(s, pos);
    if at(s, pos) == Some('/') {
        pos += 1;
    }
    (at(s, pos) == Some('>')).then_some(pos + 1)
}

/// A close tag at `k`: the place after it.
pub(crate) fn close_tag(s: &[char], k: usize) -> Option<usize> {
    if at(s, k) != Some('<') || at(s, k + 1) != Some('/') {
        return None;
    }
    let pos = skip_space(s, tag_name(s, k + 2)?);
    (at(s, pos) == Some('>')).then_some(pos + 1)
}

/// The first place `needle` occurs at or after `from`: the place after it.
fn find_after(s: &[char], from: usize, needle: &[char]) -> Option<usize> {
    let rest = s.get(from..)?;
    rest.windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p + needle.len())
}

/// A comment at `k` (`<!--->`, `<!-->`, or `<!--` ... `-->` with no `--` inside but
/// before `>`): the place after it.
fn comment(s: &[char], k: usize) -> Option<usize> {
    let open: Vec<char> = "<!--".chars().collect();
    if s.get(k..k + 4)? != open.as_slice() {
        return None;
    }
    let mut pos = k + 4;
    if at(s, pos) == Some('>') {
        return Some(pos + 1);
    }
    if at(s, pos) == Some('-') && at(s, pos + 1) == Some('>') {
        return Some(pos + 2);
    }
    loop {
        match (at(s, pos), at(s, pos + 1), at(s, pos + 2)) {
            (Some(c), _, _) if c != '-' => pos += 1,
            (Some('-'), Some(n), _) if n != '-' => pos += 2,
            (Some('-'), Some('-'), Some(t)) if t != '>' => pos += 3,
            _ => break,
        }
    }
    (at(s, pos) == Some('-') && at(s, pos + 1) == Some('-') && at(s, pos + 2) == Some('>')).then_some(pos + 3)
}

/// Any HTML markdown-it takes inline at `k` (`HTML_TAG_RE.match`): the place after it.
pub(crate) fn tag(s: &[char], k: usize) -> Option<usize> {
    if at(s, k) != Some('<') {
        return None;
    }
    if let Some(end) = open_tag(s, k).or_else(|| close_tag(s, k)).or_else(|| comment(s, k)) {
        return Some(end);
    }
    match at(s, k + 1) {
        Some('?') => find_after(s, k + 2, &['?', '>']),
        Some('!') if at(s, k + 2).is_some_and(|c| c.is_ascii_alphabetic()) => find_after(s, k + 3, &['>']),
        Some('!') => {
            let cdata: Vec<char> = "<![CDATA[".chars().collect();
            (s.get(k..k + 9)? == cdata.as_slice())
                .then(|| find_after(s, k + 9, &[']', ']', '>']))
                .flatten()
        }
        _ => None,
    }
}
