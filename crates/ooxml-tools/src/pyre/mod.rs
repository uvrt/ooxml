//! Python's regular expressions (`re`, on `str`) for the tools that take one (`find_text`'s
//! and `replace_text`'s `regex`): a pattern read as Python reads it, written in
//! `fancy-regex`'s syntax with the same meaning ([`translate`]), and matched as
//! `re.finditer` finds it, empty matches left out ([`PyRegex::find`]).
//!
//! Owns: which of Python's patterns are matched here -- literals and escapes (octal ones
//! too), `.`, classes (Python's `\w`, `\s`, `\b` and `$` written out as Python means them,
//! Unicode or ASCII), groups (named ones numbered), alternation, greedy, lazy and possessive
//! repeats, lookaround, atomic groups, back references, conditionals, the flags `i`, `m`,
//! `s`, `x`, `a` and `u`, comments -- Python's refusal of a pattern it refuses
//! ([`PatternError::Invalid`]), and the refusal of one whose meaning here would differ
//! ([`PatternError::Unported`]: `a` with `i`, `\N{...}`, the flag `t`); `re.finditer`'s
//! steps past an empty match (Python 3.7 and later: the next match may start where an
//! empty one ended, but must not be empty there).
//!
//! Must not: answer differently from Python: a pattern not certainly the same is refused,
//! never approximated. Offsets are in characters (Python's `str` indices).

mod class;
mod template;
mod translate;

use template::Piece;

pub use translate::{translate, Translated};

use std::collections::HashMap;
use std::fmt;

/// Why a pattern is not matched here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternError {
    /// Python refuses it too, with `re.error`: its message, as Python words it (with the
    /// position).
    Invalid(String),
    /// Python refuses it too, with `OverflowError` (a repeat's count too large).
    Overflow(String),
    /// Python matches it, but not certainly as this engine would; or a search gave up.
    Unported(String),
}

impl PatternError {
    /// Why, in words.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            PatternError::Invalid(why) | PatternError::Overflow(why) | PatternError::Unported(why) => why,
        }
    }

    /// The name of the exception Python raises for the pattern (`error`, `OverflowError`);
    /// `None` where Python matches it.
    #[must_use]
    pub fn python_exception(&self) -> Option<&'static str> {
        match self {
            PatternError::Invalid(_) => Some("error"),
            PatternError::Overflow(_) => Some("OverflowError"),
            PatternError::Unported(_) => None,
        }
    }

    pub(crate) fn unported(why: &str) -> PatternError {
        PatternError::Unported(why.to_owned())
    }
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for PatternError {}

/// A Python pattern, compiled.
#[derive(Debug, Clone)]
pub struct PyRegex {
    /// The pattern.
    search: fancy_regex::Regex,
    /// The pattern where a search starts, and not empty there (`re.finditer`'s step past
    /// an empty match): `\G(?:...)(?!\G)`.
    advance: fancy_regex::Regex,
    /// How many capturing groups the pattern has, and its named groups' numbers.
    groups: usize,
    names: HashMap<String, usize>,
}

/// The backtracking steps one search may take before it gives up (fancy-regex's own
/// limit): a pattern past it is refused, never answered in part.
const BACKTRACK_LIMIT: usize = 1_000_000;

/// One match: its start and end (bytes) and its groups (`None`: took no part).
type Found<'t> = fancy_regex::Captures<'t>;

impl PyRegex {
    /// `pattern` compiled as Python's `re.compile(pattern)` (`ignore_case`: with
    /// `re.IGNORECASE`).
    ///
    /// # Errors
    /// [`PatternError`], saying why: a pattern Python refuses, or one whose meaning here
    /// would differ.
    pub fn new(pattern: &str, ignore_case: bool) -> Result<PyRegex, PatternError> {
        let Translated { source, groups, names } = translate(pattern, ignore_case)?;
        let source = if ignore_case { format!("(?i){source}") } else { source };
        let build = |source: &str| {
            fancy_regex::RegexBuilder::new(source)
                .backtrack_limit(BACKTRACK_LIMIT)
                .build()
                .map_err(|e| {
                    PatternError::Unported(format!("the pattern is not one this engine reads as Python does ({e})"))
                })
        };
        Ok(PyRegex {
            search: build(&source)?,
            advance: build(&format!(r"\G(?:{source})(?!\G)"))?,
            groups,
            names,
        })
    }

    /// Where the pattern matches in `text` (as [`PyRegex::find`]), each match with
    /// `template` expanded as Python's `match.expand` expands it: `\1`..`\99` and `\g<n>`
    /// or `\g<name>` a group's text (empty when it did not take part), the escapes `\n`,
    /// `\t`, `\r`, `\f`, `\v`, `\a`, `\b`, `\\` and octal ones, any other non-letter kept
    /// with its backslash. The template is read at the first match, as Python reads it.
    ///
    /// # Errors
    /// [`PatternError`] for a template Python refuses (an unknown group, a bad escape) when
    /// there is a match, or a search that gives up.
    pub fn find_expanded(&self, text: &str, template: &str) -> Result<Vec<(usize, usize, String)>, PatternError> {
        let mut pieces: Option<Vec<Piece>> = None;
        let mut out = Vec::new();
        for (start, end, found) in self.matches(text)? {
            let pieces = match &mut pieces {
                Some(p) => p,
                None => pieces.insert(template::parse(template, self.groups, &self.names)?),
            };
            let mut with = String::new();
            for piece in pieces.iter() {
                match piece {
                    Piece::Text(t) => with.push_str(t),
                    Piece::Group(n) => with.push_str(found.get(*n).map_or("", |m| m.as_str())),
                }
            }
            out.push((start, end, with));
        }
        Ok(out)
    }

    /// Where the pattern matches in `text`, left to right and not overlapping, as
    /// `re.finditer` finds it, empty matches left out: each match's start and end in
    /// characters.
    ///
    /// # Errors
    /// [`PatternError::Unported`] where a search would take more than a million
    /// backtracking steps.
    pub fn find(&self, text: &str) -> Result<Vec<(usize, usize)>, PatternError> {
        Ok(self.matches(text)?.into_iter().map(|(s, e, _)| (s, e)).collect())
    }

    /// `re.finditer`'s matches, the empty ones left out: start and end in characters, and
    /// the captures.
    fn matches<'t>(&self, text: &'t str) -> Result<Vec<(usize, usize, Found<'t>)>, PatternError> {
        let gave_up = |e: fancy_regex::Error| PatternError::Unported(format!("the search gave up ({e})"));
        let mut out = Vec::new();
        let (mut chars, mut counted) = (0, 0);
        let mut at = 0;
        let mut after_empty = false;
        while at <= text.len() {
            let found = if after_empty {
                // Python: not empty here, else a match from the next character on
                match self.advance.captures_from_pos(text, at).map_err(gave_up)? {
                    Some(found) => Some(found),
                    None => match text.get(at..).and_then(|rest| rest.chars().next()) {
                        Some(c) => self
                            .search
                            .captures_from_pos(text, at + c.len_utf8())
                            .map_err(gave_up)?,
                        None => None,
                    },
                }
            } else {
                self.search.captures_from_pos(text, at).map_err(gave_up)?
            };
            let Some(found) = found else { break };
            let Some(whole) = found.get(0) else { break };
            let (s, e) = (whole.start(), whole.end());
            after_empty = s == e;
            at = e;
            if s == e {
                continue;
            }
            let start = chars + text.get(counted..s).map_or(0, |t| t.chars().count());
            let end = start + whole.as_str().chars().count();
            (chars, counted) = (end, e);
            out.push((start, end, found));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::PyRegex;

    fn find(pattern: &str, text: &str) -> Vec<(usize, usize)> {
        PyRegex::new(pattern, false)
            .and_then(|r| r.find(text))
            .unwrap_or_else(|e| panic!("{pattern}: {e}"))
    }

    fn refused(pattern: &str) -> bool {
        PyRegex::new(pattern, false).is_err()
    }

    #[test]
    fn matches_are_python_s_in_characters() {
        assert_eq!(find(r"\d+", "é 12 and 345"), vec![(2, 4), (9, 12)]);
        assert_eq!(find("a|ab", "abab"), vec![(0, 1), (2, 3)]);
        assert_eq!(find("(?i)straße", "STRASSE Straße"), vec![(8, 14)]);
        assert_eq!(find(r"\bcat\b", "cat concat cat_ cat."), vec![(0, 3), (16, 19)]);
        assert_eq!(find(r"x$", "ax\n"), vec![(1, 2)]);
        assert_eq!(find(r"x$", "ax\nx"), vec![(3, 4)]);
        assert_eq!(find(r"(?m)x$", "ax\nx"), vec![(1, 2), (3, 4)]);
        assert_eq!(find(r"\s", "a\u{1c}b c"), vec![(1, 2), (3, 4)]);
        assert_eq!(find(r"\w+", "x²y\u{301}z"), vec![(0, 3), (4, 5)]);
        assert_eq!(find("a{x}", "a{x}"), vec![(0, 4)]);
        assert_eq!(find("a{,2}b", "aaab"), vec![(1, 4)]);
        assert_eq!(find("[]a-c[]+", "x]b[c"), vec![(1, 5)]);
        assert_eq!(find("[a&&b]", "&"), vec![(0, 1)]);
        assert_eq!(find(r"(?P<q>['])x(?P=q)", "'x' \"x\""), vec![(0, 3)]);
        assert_eq!(find(r"a++b|a", "aab"), vec![(0, 3)]);
        assert_eq!(find(r"(?<=\$)\d+", "$12 34"), vec![(1, 3)]);
        assert_eq!(find(r"\x41é\.", "Aé."), vec![(0, 3)]);
        assert_eq!(find(r"(?#note)ab", "ab"), vec![(0, 2)]);
        assert_eq!(find(r"\<a\>", "<a>"), vec![(0, 3)]);
    }

    #[test]
    fn what_python_refuses_or_reads_otherwise_is_refused() {
        for pattern in [
            r"\p{L}",
            r"\N{BULLET}",
            r"\12",
            "(",
            ")",
            "*a",
            "a**",
            "a(?i)b",
            r"\q",
            r"[\B]",
            r"\1(a)",
            "(?<n>a)",
            "[a-",
        ] {
            assert!(refused(pattern), "{pattern} should be refused");
        }
    }
}
