//! Owns: markdown-it's link helpers (`helpers/`, `common/utils.py`): a link destination
//! and title read, backslash escapes and entities unescaped, a reference label
//! normalised.
//!
//! Must not: normalise or validate URLs (that is [`crate::url`]).

use crate::chars::{is_trim, md_trim};
use std::sync::OnceLock;

/// A destination read: its text and the place after it.
pub(crate) struct Destination {
    pub(crate) text: String,
    pub(crate) pos: usize,
}

/// A link destination at `pos` of `s` (up to `maximum`): `<...>`, or a run without
/// spaces or controls and with balanced parentheses (`parseLinkDestination`).
pub(crate) fn destination(s: &[char], mut pos: usize, maximum: usize) -> Option<Destination> {
    let start = pos;
    let at = |k: usize| s.get(k).copied();
    if at(pos) == Some('<') {
        pos += 1;
        while pos < maximum {
            match at(pos)? {
                '\n' | '<' => return None,
                '>' => {
                    return Some(Destination {
                        text: unescape_all(&collect(s, start + 1, pos)),
                        pos: pos + 1,
                    })
                }
                '\\' if pos + 1 < maximum => {
                    pos += 2;
                    continue;
                }
                _ => pos += 1,
            }
        }
        return None;
    }
    let mut level = 0;
    while pos < maximum {
        let Some(code) = at(pos) else { break };
        if code == ' ' || u32::from(code) < 0x20 || code == '\u{7f}' {
            break;
        }
        if code == '\\' && pos + 1 < maximum {
            if at(pos + 1) == Some(' ') {
                break;
            }
            pos += 2;
            continue;
        }
        if code == '(' {
            level += 1;
            if level > 32 {
                return None;
            }
        }
        if code == ')' {
            if level == 0 {
                break;
            }
            level -= 1;
        }
        pos += 1;
    }
    if start == pos || level != 0 {
        return None;
    }
    Some(Destination {
        text: unescape_all(&collect(s, start, pos)),
        pos,
    })
}

/// A link title being read (`parseLinkTitle`'s state): complete, or to be continued on
/// the next line.
#[derive(Debug, Clone, Default)]
pub(crate) struct Title {
    pub(crate) ok: bool,
    pub(crate) can_continue: bool,
    pub(crate) pos: usize,
    pub(crate) text: String,
    marker: char,
}

/// A link title at `start` of `s` (`"..."`, `'...'`, `(...)`), or the one `previous`
/// began, continued.
pub(crate) fn title(s: &[char], mut start: usize, maximum: usize, previous: Option<&Title>) -> Title {
    let mut pos = start;
    let mut state = Title::default();
    if let Some(previous) = previous {
        state.text.clone_from(&previous.text);
        state.marker = previous.marker;
    } else {
        if pos >= maximum {
            return state;
        }
        let marker = match s.get(pos) {
            Some('"') => '"',
            Some('\'') => '\'',
            Some('(') => ')',
            _ => return state,
        };
        start += 1;
        pos += 1;
        state.marker = marker;
    }
    while pos < maximum {
        let code = s.get(pos).copied();
        if code == Some(state.marker) {
            state.pos = pos + 1;
            state.text.push_str(&unescape_all(&collect(s, start, pos)));
            state.ok = true;
            return state;
        } else if code == Some('(') && state.marker == ')' {
            return state;
        } else if code == Some('\\') && pos + 1 < maximum {
            pos += 1;
        }
        pos += 1;
    }
    state.can_continue = true;
    state.text.push_str(&unescape_all(&collect(s, start, pos)));
    state
}

/// The characters `[start, end)` of `s`.
pub(crate) fn collect(s: &[char], start: usize, end: usize) -> String {
    s.get(start..end.max(start)).unwrap_or_default().iter().collect()
}

/// The HTML5 named character references, sorted by name.
fn entities() -> &'static [(&'static str, String)] {
    static TABLE: OnceLock<Vec<(&'static str, String)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("../data/entities.tsv")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|line| {
                let (name, codes) = line.split_once('\t')?;
                let text = codes
                    .split(' ')
                    .filter_map(|c| u32::from_str_radix(c, 16).ok().and_then(char::from_u32))
                    .collect();
                Some((name, text))
            })
            .collect()
    })
}

/// The characters an entity name stands for.
pub(crate) fn entity(name: &str) -> Option<&'static str> {
    let table = entities();
    table
        .binary_search_by(|(n, _)| (*n).cmp(name))
        .ok()
        .and_then(|k| table.get(k))
        .map(|(_, text)| text.as_str())
}

/// Whether a numeric reference's code point is one a document may hold
/// (`isValidEntityCode`).
pub(crate) fn valid_code(c: u32) -> bool {
    !((0xD800..=0xDFFF).contains(&c)
        || (0xFDD0..=0xFDEF).contains(&c)
        || (c & 0xFFFF) == 0xFFFF
        || (c & 0xFFFF) == 0xFFFE
        || c <= 0x08
        || c == 0x0B
        || (0x0E..=0x1F).contains(&c)
        || (0x7F..=0x9F).contains(&c)
        || c > 0x10FFFF)
}

/// What `&name;` stands for: a named reference's characters, a valid numeric one's
/// character, else itself (`replaceEntityPattern`).
pub(crate) fn replace_entity(whole: &str, name: &str) -> String {
    if let Some(text) = entity(name) {
        return text.to_owned();
    }
    let lower = name.to_ascii_lowercase();
    let code = if let Some(hex) = lower.strip_prefix("#x") {
        (!hex.is_empty() && hex.len() <= 8 && hex.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| u32::from_str_radix(hex, 16).ok())
            .flatten()
    } else if let Some(digits) = lower.strip_prefix('#') {
        (!digits.is_empty() && digits.len() <= 8 && digits.chars().all(|c| c.is_ascii_digit()))
            .then(|| digits.parse::<u32>().ok())
            .flatten()
    } else {
        None
    };
    match code.filter(|c| valid_code(*c)).and_then(char::from_u32) {
        Some(ch) => ch.to_string(),
        None => whole.to_owned(),
    }
}

/// Backslash escapes and entity references replaced (`unescapeAll`).
pub(crate) fn unescape_all(text: &str) -> String {
    if !text.contains(['\\', '&']) {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut k = 0;
    while let Some(&ch) = chars.get(k) {
        if ch == '\\' {
            if let Some(next) = chars.get(k + 1).filter(|c| c.is_ascii_punctuation()) {
                out.push(*next);
                k += 2;
                continue;
            }
        } else if ch == '&' {
            if let Some((name, end)) = entity_name(&chars, k + 1) {
                let whole = collect(&chars, k, end);
                out.push_str(&replace_entity(&whole, &name));
                k = end;
                continue;
            }
        }
        out.push(ch);
        k += 1;
    }
    out
}

/// `([a-z#][a-z0-9]{1,31});` at `from`, ignoring case: the name and the place after `;`.
fn entity_name(chars: &[char], from: usize) -> Option<(String, usize)> {
    let first = chars.get(from).filter(|c| c.is_ascii_alphabetic() || **c == '#')?;
    let mut end = from + 1;
    while chars.get(end).is_some_and(char::is_ascii_alphanumeric) && end - from < 32 {
        end += 1;
    }
    if end - from < 2 || chars.get(end) != Some(&';') {
        return None;
    }
    let _ = first;
    Some((collect(chars, from, end), end + 1))
}

/// A reference label as markdown-it compares them: trimmed, white space collapsed, case
/// folded by lower- then upper-casing (`normalizeReference`).
pub(crate) fn normalize_reference(label: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for ch in md_trim(label).chars() {
        if is_trim(ch) {
            space = true;
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(ch);
    }
    out.to_lowercase().to_uppercase()
}
