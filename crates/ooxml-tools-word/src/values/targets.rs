//! Owns: the forms of a scale's targets that name many values (`ooxml_edit.tools.scales`'
//! `column_target`, `reference_target`, `reference_matches`): a table's column
//! (`t:X/col2`) and the values written by reference (`{{d:scores/*/score}}`), and whether a
//! reference a value was written from is one such a target names (`fnmatch.fnmatchcase` per
//! part between `/`).
//!
//! Must not: resolve a target in a document.

use super::scale::py_strip;

/// `(table, column)` for a table column target (`<table>/col2`), else `None`.
pub(crate) fn column_target(target: &str) -> Option<(&str, usize)> {
    let target = py_strip(target);
    let at = target.rfind("/col")?;
    let (table, digits) = (&target[..at], &target[at + 4..]);
    if table.is_empty() || table.contains('\n') || digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((table, digits.parse().ok()?))
}

/// The pattern of a `{{...}}` target (`d:scores/*/score`), else `None`.
pub(crate) fn reference_target(target: &str) -> Option<&str> {
    let inner = py_strip(target).strip_prefix("{{")?.strip_suffix("}}")?;
    if inner.contains(['{', '}']) {
        return None;
    }
    Some(inner.trim_matches(|c: char| c.is_whitespace()))
}

/// Whether a value written from `source` is one `wanted` names: the same reference, a
/// format after `:` aside, case aside; `*` stands for any one part between `/`, a `wanted`
/// of `*` alone for every reference.
pub(crate) fn reference_matches(wanted: &str, source: &str) -> bool {
    let (wanted, source) = (bare(wanted), bare(source));
    if wanted == "*" || wanted.is_empty() {
        return true;
    }
    let parts: Vec<&str> = wanted.split('/').collect();
    let have: Vec<&str> = source.split('/').collect();
    parts.len() == have.len() && parts.iter().zip(&have).all(|(p, h)| fnmatch(h, p))
}

/// A reference without its format (`d:x/a/b:0.0` -> `d:x/a/b`), case folded (`_bare`).
fn bare(reference: &str) -> String {
    let text = py_strip(reference);
    let out = match text.split_once(':') {
        Some((head, rest)) if rest.contains('/') => {
            let (front, last) = rest.rsplit_once('/').unwrap_or(("", rest));
            let tail = last.split(':').next().unwrap_or("");
            format!("{head}:{front}/{tail}")
        }
        _ => text.to_owned(),
    };
    ooxml_word_model::docx::casefold(&out)
}

/// Python's `fnmatch.fnmatchcase(name, pattern)`: `*`, `?`, `[seq]` and `[!seq]`.
fn fnmatch(name: &str, pattern: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    matches_from(&name, &pattern)
}

fn matches_from(name: &[char], pattern: &[char]) -> bool {
    let Some((&first, rest)) = pattern.split_first() else {
        return name.is_empty();
    };
    let one = |wanted: &dyn Fn(char) -> bool, after: &[char]| {
        name.split_first()
            .is_some_and(|(c, more)| wanted(*c) && matches_from(more, after))
    };
    match first {
        '*' => (0..=name.len()).any(|k| name.get(k..).is_some_and(|tail| matches_from(tail, rest))),
        '?' => one(&|_| true, rest),
        '[' => match class(rest) {
            Some((members, negated, after)) => one(&|c| in_class(members, c) != negated, after),
            // an unclosed `[` is itself
            None => one(&|c| c == '[', rest),
        },
        wanted => one(&|c| c == wanted, rest),
    }
}

/// A `[...]` class after its `[`: its members, whether it is negated (`[!...]`), and what
/// follows its `]`; `None` where it is not closed.
fn class(pattern: &[char]) -> Option<(&[char], bool, &[char])> {
    let (negated, body) = match pattern.split_first() {
        Some(('!', rest)) => (true, rest),
        _ => (false, pattern),
    };
    // a `]` first is one of the class
    let close = body.iter().skip(1).position(|&c| c == ']')? + 1;
    let (members, after) = body.split_at_checked(close)?;
    Some((members, negated, after.get(1..).unwrap_or_default()))
}

/// Whether `c` is among a class's `members` (single characters and ranges `a-z`).
fn in_class(members: &[char], c: char) -> bool {
    let mut left = members;
    while let Some((&first, rest)) = left.split_first() {
        match rest {
            ['-', last, more @ ..] => {
                if first <= c && c <= *last {
                    return true;
                }
                left = more;
            }
            _ => {
                if first == c {
                    return true;
                }
                left = rest;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_column_and_a_reference_are_read_as_scales_read_them() {
        assert_eq!(column_target(" t:0A1B/col2 "), Some(("t:0A1B", 2)));
        assert_eq!(column_target("t:0A1B/c1,2"), None);
        assert_eq!(column_target("/col2"), None);
        assert_eq!(reference_target("{{ d:scores/*/score }}"), Some("d:scores/*/score"));
        assert_eq!(reference_target("{{d:{x}}}"), None);
        assert_eq!(reference_target("p:A"), None);
    }

    #[test]
    fn a_reference_matches_by_its_parts_its_format_and_case_aside() {
        assert!(reference_matches("d:scores/*/score", "d:Scores/A/score:0.0"));
        assert!(reference_matches("*", "d:x/y"));
        assert!(!reference_matches("d:scores/*", "d:scores/A/score"));
        assert!(reference_matches("d:s/[ab]/x", "d:s/b/x"));
        assert!(!reference_matches("d:s/[!ab]/x", "d:s/b/x"));
        assert!(reference_matches("d:s/a?/x", "d:s/ab/x"));
    }
}
