//! Python's regular expressions (`re`, on `str`) for the tools that take one (`find_text`'s
//! `regex`): a pattern read as Python reads it, written in `fancy-regex`'s syntax with the
//! same meaning ([`translate`]), and matched as `re.finditer` finds it, empty matches left
//! out ([`PyRegex::find`]).
//!
//! Owns: which of Python's patterns are matched here -- every construct whose meaning is
//! the same once written so: literals and escapes, `.`, classes (Python's `\w`, `\s`,
//! `\b` and `$` written out as Python means them), groups (named ones numbered),
//! alternation, greedy, lazy and possessive repeats, lookaround, atomic groups, back
//! references, the flags `i`, `m` and `s`, comments -- and the refusal, with its reason, of
//! every other: one Python refuses (Rust must not answer where Python fails), one whose
//! meaning would differ (`x`, `a`, `L`, `u`, conditionals, octal and two-digit escapes,
//! `\N{...}`), and one that can match the empty string (`re.finditer` then resumes at the
//! same place, which no Rust engine does, so the matches after it could differ).
//!
//! Must not: answer differently from Python: a pattern not certainly the same is refused,
//! never approximated. Offsets are in characters (Python's `str` indices).

mod translate;

pub use translate::translate;

use std::fmt;

/// Why a pattern is not matched here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported(pub String);

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unsupported {}

/// A Python pattern, compiled.
#[derive(Debug, Clone)]
pub struct PyRegex {
    regex: fancy_regex::Regex,
}

/// The backtracking steps one search may take before it gives up (fancy-regex's own
/// limit): a pattern past it is refused, never answered in part.
const BACKTRACK_LIMIT: usize = 1_000_000;

impl PyRegex {
    /// `pattern` compiled as Python's `re.compile(pattern)` (`ignore_case`: with
    /// `re.IGNORECASE`).
    ///
    /// # Errors
    /// [`Unsupported`], saying why: a pattern Python refuses, one whose meaning here would
    /// differ, or one that can match the empty string.
    pub fn new(pattern: &str, ignore_case: bool) -> Result<PyRegex, Unsupported> {
        let translated = translate(pattern)?;
        let source = if ignore_case {
            format!("(?i){translated}")
        } else {
            translated
        };
        let regex = fancy_regex::RegexBuilder::new(&source)
            .backtrack_limit(BACKTRACK_LIMIT)
            .build()
            .map_err(|e| Unsupported(format!("the pattern is not one this engine reads as Python does ({e})")))?;
        Ok(PyRegex { regex })
    }

    /// Where the pattern matches in `text`, left to right and not overlapping, as
    /// `re.finditer` finds it (the pattern never matches the empty string): each match's
    /// start and end in characters.
    ///
    /// # Errors
    /// [`Unsupported`] where a search would take more than [`BACKTRACK_LIMIT`] steps.
    pub fn find(&self, text: &str) -> Result<Vec<(usize, usize)>, Unsupported> {
        let mut out = Vec::new();
        let mut chars = 0;
        let mut counted = 0;
        for found in self.regex.find_iter(text) {
            let found = found.map_err(|e| Unsupported(format!("the search gave up ({e})")))?;
            let start = chars + text.get(counted..found.start()).map_or(0, |s| s.chars().count());
            let end = start + found.as_str().chars().count();
            out.push((start, end));
            chars = end;
            counted = found.end();
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
            "a*", "(?x)a b", "(?a)\\w", r"\p{L}", r"\N{BULLET}", r"\12", r"\0", "(?(1)a|b)", "(", ")", "*a",
            "a**", "a(?i)b", r"\q", r"[\B]", r"\1(a)", "(?<n>a)", "[a-",
        ] {
            assert!(refused(pattern), "{pattern} should be refused");
        }
    }
}
