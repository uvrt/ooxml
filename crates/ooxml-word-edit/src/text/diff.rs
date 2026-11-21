//! Owns: the diffs set_text writes by -- Python's
//! `difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes()` ([`opcodes`], from
//! [`super::matcher`]'s matching blocks, bounded there for long unlike texts),
//! and docx-agent's two foldings of it: [`coarse`] (characters; short coincidental matches
//! folded into the change around them) and [`by_words`] (a tracked replacement's stretches
//! at word boundaries); and its [`ratio`], how alike two texts are.
//!
//! Must not: be used to find what a change touched (the change set says that); this is the
//! edit's own rule for where new characters go.

use std::hash::Hash;

use super::matcher::matching_blocks;

/// What an opcode does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tag {
    /// `a[i1..i2]` is `b[j1..j2]`.
    Equal,
    /// Anything else (replace, delete, insert; the foldings do not tell them apart).
    Change,
}

/// An opcode: the tag and the ranges `a[i1..i2]`, `b[j1..j2]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Opcode {
    pub(crate) tag: Tag,
    pub(crate) i1: usize,
    pub(crate) i2: usize,
    pub(crate) j1: usize,
    pub(crate) j2: usize,
}

/// `difflib.SequenceMatcher(None, a, b, autojunk=False).ratio()`: twice the characters
/// the matching blocks hold over both lengths (1 for two empty sequences); past
/// [`super::matcher::WORK`], the common start and end only.
#[must_use]
pub fn ratio<T: Eq + Hash>(a: &[T], b: &[T]) -> f64 {
    let length = a.len() + b.len();
    if length == 0 {
        return 1.0;
    }
    let matches: usize = matching_blocks(a, b).iter().map(|(_, _, k)| k).sum();
    2.0 * matches as f64 / length as f64
}

/// `difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes()`, replace, delete
/// and insert all [`Tag::Change`]; past [`super::matcher::WORK`], the common start and end
/// kept and one change between.
pub(crate) fn opcodes<T: Eq + Hash>(a: &[T], b: &[T]) -> Vec<Opcode> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    for (ai, bj, size) in matching_blocks(a, b) {
        if i < ai || j < bj {
            out.push(Opcode {
                tag: Tag::Change,
                i1: i,
                i2: ai,
                j1: j,
                j2: bj,
            });
        }
        (i, j) = (ai + size, bj + size);
        if size > 0 {
            out.push(Opcode {
                tag: Tag::Equal,
                i1: ai,
                i2: i,
                j1: bj,
                j2: j,
            });
        }
    }
    out
}

/// Characters below this length that happen to match inside a changed stretch are folded
/// into the change (pptx-agent's rule and threshold).
const MIN_KEPT: usize = 3;

/// Fold `raw` opcodes: an interior equal stretch for which `fold` says so becomes part of
/// the change around it, and adjacent changes join; `at_a`/`at_b` map indexes to offsets.
fn folded(
    raw: &[Opcode],
    fold: impl Fn(&Opcode) -> bool,
    at_a: impl Fn(usize) -> usize,
    at_b: impl Fn(usize) -> usize,
) -> Vec<Opcode> {
    let mut out: Vec<Opcode> = Vec::new();
    for (index, op) in raw.iter().enumerate() {
        let interior = index > 0 && index + 1 < raw.len();
        let mut tag = op.tag;
        if tag == Tag::Equal && interior && fold(op) {
            tag = Tag::Change;
        }
        if tag == Tag::Change {
            if let Some(last) = out.last_mut().filter(|l| l.tag == Tag::Change) {
                last.i2 = at_a(op.i2);
                last.j2 = at_b(op.j2);
                continue;
            }
        }
        out.push(Opcode {
            tag,
            i1: at_a(op.i1),
            i2: at_a(op.i2),
            j1: at_b(op.j1),
            j2: at_b(op.j2),
        });
    }
    out
}

/// docx-agent's `coarse_opcodes`: character opcodes with coincidental short matches folded
/// into the edits around them.
pub(crate) fn coarse(old: &[char], new: &[char]) -> Vec<Opcode> {
    folded(&opcodes(old, new), |op| op.i2 - op.i1 < MIN_KEPT, |i| i, |j| j)
}

/// Python's `\w` (`str.isalnum()` or `_`), as near as the standard library tells: letters
/// and numbers.
fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Python's `str.isspace()` for one character: Unicode white space, and the information
/// separators U+001C..U+001F.
pub(crate) fn python_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// The tokens a tracked replacement is diffed by: a word, a stretch of white space, or one
/// other character (`\w+|\s+|[^\w\s]`).
fn tokens(text: &[char]) -> Vec<&[char]> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(&c) = text.get(at) {
        let class = |x: char| {
            if word_char(x) {
                0
            } else if python_space(x) {
                1
            } else {
                2
            }
        };
        let kind = class(c);
        let mut end = at + 1;
        if kind != 2 {
            while text.get(end).is_some_and(|x| class(*x) == kind) {
                end += 1;
            }
        }
        out.extend(text.get(at..end));
        at = end;
    }
    out
}

/// docx-agent's `word_opcodes`: [`coarse`] at word boundaries, for a tracked replacement --
/// whole words deleted and inserted, unchanged words kept; white space alone between two
/// changed words is part of the change. Offsets are characters.
pub(crate) fn by_words(old: &[char], new: &[char]) -> Vec<Opcode> {
    let (a, b) = (tokens(old), tokens(new));
    let offsets = |t: &[&[char]]| {
        let mut at = vec![0];
        for token in t {
            at.push(at.last().copied().unwrap_or(0) + token.len());
        }
        at
    };
    let (at_a, at_b) = (offsets(&a), offsets(&b));
    let raw = opcodes(&a, &b);
    let all_space = |op: &Opcode| {
        a.get(op.i1..op.i2)
            .is_some_and(|t| t.iter().all(|token| token.iter().all(|c| python_space(*c))))
    };
    folded(
        &raw,
        all_space,
        |i| at_a.get(i).copied().unwrap_or(0),
        |j| at_b.get(j).copied().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn ops(found: Vec<Opcode>) -> Vec<(bool, usize, usize, usize, usize)> {
        found
            .into_iter()
            .map(|o| (o.tag == Tag::Equal, o.i1, o.i2, o.j1, o.j2))
            .collect()
    }

    #[test]
    fn as_difflib() {
        // difflib.SequenceMatcher(None, "qabxcd", "abycdf", autojunk=False).get_opcodes()
        assert_eq!(
            ops(opcodes(&chars("qabxcd"), &chars("abycdf"))),
            vec![
                (false, 0, 1, 0, 0),
                (true, 1, 3, 0, 2),
                (false, 3, 4, 2, 3),
                (true, 4, 6, 3, 5),
                (false, 6, 6, 5, 6)
            ]
        );
    }

    #[test]
    fn coarse_and_word_foldings() {
        // text.coarse_opcodes("Sample Document", "Changed Document more")
        assert_eq!(
            ops(coarse(&chars("Sample Document"), &chars("Changed Document more"))),
            vec![(false, 0, 6, 0, 7), (true, 6, 15, 7, 16), (false, 15, 15, 16, 21)]
        );
        // text.word_opcodes("Quarterly report", "Half-yearly report")
        assert_eq!(
            ops(by_words(&chars("Quarterly report"), &chars("Half-yearly report"))),
            vec![(false, 0, 9, 0, 11), (true, 9, 16, 11, 18)]
        );
    }

    /// One case of `data/difflib-cases.json`: old, new, docx-agent's `coarse_opcodes` and
    /// `word_opcodes` (equal?, i1, i2, j1, j2), as `data/difflib_cases.py` wrote them.
    type Case = (
        String,
        String,
        Vec<(bool, usize, usize, usize, usize)>,
        Vec<(bool, usize, usize, usize, usize)>,
    );

    #[test]
    fn as_python_difflib_on_random_texts() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("../../data/difflib-cases.json")).unwrap_or_default();
        assert!(cases.len() >= 200);
        for (old, new, want_coarse, want_words) in cases {
            let (a, b) = (chars(&old), chars(&new));
            assert_eq!(ops(coarse(&a, &b)), want_coarse, "coarse {old:?} {new:?}");
            assert_eq!(ops(by_words(&a, &b)), want_words, "by_words {old:?} {new:?}");
        }
    }

    /// The bench for long texts (lane D3): `PROBE_MAX=<chars> cargo test --release -p
    /// ooxml-word-edit --lib timing -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing probe"]
    fn timing_probe() {
        for n in [1000usize, 5000, 20000, 100_000] {
            let unlike_a: Vec<char> = "before text ".repeat(n / 12 + 1).chars().take(n).collect();
            let unlike_b: Vec<char> = "after text ".repeat(n / 11 + 1).chars().take(n).collect();
            let mut alike_b = unlike_a.clone();
            alike_b.splice(n / 2..n / 2, "XYZ".chars());
            // many distinct words, the second text in another order: a large vocabulary
            let words: Vec<String> = (0..n / 6).map(|k| format!("w{k} ")).collect();
            let words_a: Vec<char> = words.concat().chars().collect();
            let words_b: Vec<char> = words.iter().rev().cloned().collect::<String>().chars().collect();
            for (name, a, b) in [
                ("unlike", &unlike_a, &unlike_b),
                ("alike", &unlike_a, &alike_b),
                ("words", &words_a, &words_b),
            ] {
                let limit = std::env::var("PROBE_MAX")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(usize::MAX);
                if n > limit {
                    continue;
                }
                let t = std::time::Instant::now();
                let found = coarse(a, b).len();
                let chars = t.elapsed();
                let t = std::time::Instant::now();
                let by_word = by_words(a, b).len();
                eprintln!(
                    "PROBE {name} {n}: coarse {chars:?} ({found} opcodes), by_words {:?} ({by_word})",
                    t.elapsed()
                );
            }
        }
    }
}
