//! Owns: keys compared loosely and in key order (ooxml-edit's `tools/keys.py` `loose_key`,
//! docx-agent's `table_sync.natural`): one rule every tool that matches keys shares.
//!
//! A key is read as runs -- letters (case folded), digits 0-9 (compared as a number), or one
//! other character; white space, the invisible characters and the separators `. - _ /`
//! between two runs are one boundary whatever they are, at either end white space is
//! ignored and a separator counts.
//!
//! Must not: guess more than that (no spelling corrections, no reordering).

use ooxml_base::pystr::is_alpha;

/// The separators between two runs that are one boundary.
const SEPARATORS: &[char] = &['.', '-', '_', '/'];
/// Characters that print nothing, read as white space.
const INVISIBLE: &[char] = &['\u{200b}', '\u{200c}', '\u{200d}', '\u{2060}', '\u{feff}', '\u{ad}'];

/// One run of a key's loose form.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Run {
    /// Letters, case folded.
    Letters(String),
    /// A number: its digits without leading zeros, ordered by length first.
    Digits(usize, String),
    /// One other character.
    Other(char),
}

/// Python's `str.isspace`.
fn py_space(c: char) -> bool {
    crate::written::py_space(c)
}

fn gap(c: char) -> bool {
    py_space(c) || INVISIBLE.contains(&c)
}

/// Python's `str.casefold` for a run of letters.
fn casefold(text: &str) -> String {
    ooxml_word_model::docx::casefold(text)
}

/// `text`'s canonical form for loose key matching (`loose_key`): equal for two keys that
/// match loosely, empty for a key of white space only.
pub(crate) fn loose_key(text: &str) -> Vec<Run> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    let boundary = |c: &char| gap(*c) || SEPARATORS.contains(c);
    while let Some(&c) = chars.peek() {
        if boundary(&c) {
            let mut separators = Vec::new();
            while let Some(found) = chars.next_if(boundary) {
                if SEPARATORS.contains(&found) {
                    separators.push(Run::Other(found));
                }
            }
            // at either end white space is ignored and separators count; between two runs
            // they are one boundary, whatever it is made of
            if chars.peek().is_none() || out.is_empty() {
                out.extend(separators);
            }
        } else if c.is_ascii_digit() {
            let digits: String = std::iter::from_fn(|| chars.next_if(char::is_ascii_digit)).collect();
            let digits = digits.trim_start_matches('0');
            let digits = if digits.is_empty() { "0" } else { digits };
            out.push(Run::Digits(digits.len(), digits.to_owned()));
        } else if is_alpha(c) {
            let letters: String = std::iter::from_fn(|| chars.next_if(|l| is_alpha(*l))).collect();
            out.push(Run::Letters(casefold(&letters)));
        } else {
            chars.next();
            out.push(Run::Other(c));
        }
    }
    out
}

/// One part of a key's sort order (`natural`): a run of digits as its number (before any
/// text, as Python's `(0, n, "")` sorts before `(1, 0, text)`), the rest as text with its
/// case folded.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Part {
    /// A number (beyond `u128`, the largest).
    Number(u128),
    /// Text, case folded.
    Text(String),
}

/// A key's sort order (docx-agent's `table_sync.natural`): its runs of digits as numbers,
/// the rest as text, the case aside.
pub(crate) fn natural(key: &str) -> Vec<Part> {
    let mut out = Vec::new();
    let mut run = String::new();
    let mut digits = false;
    let mut flush = |run: &mut String, digits: bool| {
        if run.is_empty() {
            return;
        }
        let text = std::mem::take(run);
        out.push(if digits {
            Part::Number(
                text.chars()
                    .filter_map(|c| c.to_digit(10))
                    .fold(0u128, |n, d| n.saturating_mul(10).saturating_add(u128::from(d))),
            )
        } else {
            Part::Text(casefold(&text))
        });
    };
    for c in key.chars() {
        let is = c.is_ascii_digit();
        if is != digits {
            flush(&mut run, digits);
            digits = is;
        }
        run.push(c);
    }
    flush(&mut run, digits);
    out
}

#[cfg(test)]
mod tests {
    use super::{loose_key, natural};

    #[test]
    fn keys_spelled_two_ways_match_loosely() {
        assert_eq!(loose_key("K7-12.3"), loose_key("k7_12_03"));
        assert_eq!(loose_key("K7 - 12"), loose_key("k7/12"));
        assert_eq!(loose_key("K7"), loose_key("K-7"));
        assert_ne!(loose_key("03-1"), loose_key("031"));
        assert_ne!(loose_key("AB"), loose_key("A B"));
        assert_ne!(loose_key("K7-"), loose_key("K7"));
        assert_eq!(loose_key(" K7\u{200b}"), loose_key("k7"));
        assert!(loose_key(" \u{a0} ").is_empty());
    }

    #[test]
    fn keys_sort_with_their_numbers_as_numbers() {
        assert!(natural("K-9") < natural("K-10"));
        assert!(natural("a2") < natural("B1"));
        assert!(natural("10") < natural("x"));
        assert!(natural("Q7-001") < natural("Q7-002"));
    }
}
