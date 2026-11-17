//! Owns: the diffs set_text writes by -- Python's
//! `difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes()` ported step for step
//! ([`opcodes`]: the same longest matches, found in the same order, joined the same way),
//! and docx-agent's two foldings of it: [`coarse`] (characters; short coincidental matches
//! folded into the change around them) and [`by_words`] (a tracked replacement's stretches
//! at word boundaries).
//!
//! Must not: be used to find what a change touched (the change set says that); this is the
//! edit's own rule for where new characters go.

use std::collections::HashMap;
use std::hash::Hash;

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

struct Matcher<'s, T> {
    a: &'s [T],
    b: &'s [T],
    b2j: HashMap<&'s T, Vec<usize>>,
}

impl<'s, T: Eq + Hash> Matcher<'s, T> {
    fn new(a: &'s [T], b: &'s [T]) -> Self {
        let mut b2j: HashMap<&T, Vec<usize>> = HashMap::new();
        for (j, x) in b.iter().enumerate() {
            b2j.entry(x).or_default().push(j);
        }
        Matcher { a, b, b2j }
    }

    /// `find_longest_match(alo, ahi, blo, bhi)`.
    fn longest(&self, alo: usize, ahi: usize, blo: usize, bhi: usize) -> (usize, usize, usize) {
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0usize);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        let mut newj2len: HashMap<usize, usize> = HashMap::new();
        for (i, x) in self.a.iter().enumerate().take(ahi).skip(alo) {
            newj2len.clear();
            for &j in self.b2j.get(x).map_or(&[][..], Vec::as_slice) {
                if j < blo {
                    continue;
                }
                if j >= bhi {
                    break;
                }
                let k = j.checked_sub(1).and_then(|p| j2len.get(&p)).copied().unwrap_or(0) + 1;
                newj2len.insert(j, k);
                if k > bestsize {
                    (besti, bestj, bestsize) = (i + 1 - k, j + 1 - k, k);
                }
            }
            std::mem::swap(&mut j2len, &mut newj2len);
        }
        let same = |i: usize, j: usize| matches!((self.a.get(i), self.b.get(j)), (Some(x), Some(y)) if x == y);
        while besti > alo && bestj > blo && same(besti - 1, bestj - 1) {
            (besti, bestj, bestsize) = (besti - 1, bestj - 1, bestsize + 1);
        }
        while besti + bestsize < ahi && bestj + bestsize < bhi && same(besti + bestsize, bestj + bestsize) {
            bestsize += 1;
        }
        (besti, bestj, bestsize)
    }

    /// `get_matching_blocks()`.
    fn matching_blocks(&self) -> Vec<(usize, usize, usize)> {
        let (la, lb) = (self.a.len(), self.b.len());
        let mut queue = vec![(0, la, 0, lb)];
        let mut blocks = Vec::new();
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let (i, j, k) = self.longest(alo, ahi, blo, bhi);
            if k > 0 {
                blocks.push((i, j, k));
                if alo < i && blo < j {
                    queue.push((alo, i, blo, j));
                }
                if i + k < ahi && j + k < bhi {
                    queue.push((i + k, ahi, j + k, bhi));
                }
            }
        }
        blocks.sort_unstable();
        let (mut i1, mut j1, mut k1) = (0, 0, 0);
        let mut out = Vec::new();
        for (i2, j2, k2) in blocks {
            if i1 + k1 == i2 && j1 + k1 == j2 {
                k1 += k2;
            } else {
                if k1 > 0 {
                    out.push((i1, j1, k1));
                }
                (i1, j1, k1) = (i2, j2, k2);
            }
        }
        if k1 > 0 {
            out.push((i1, j1, k1));
        }
        out.push((la, lb, 0));
        out
    }
}

/// `difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes()`, replace, delete
/// and insert all [`Tag::Change`].
pub(crate) fn opcodes<T: Eq + Hash>(a: &[T], b: &[T]) -> Vec<Opcode> {
    let matcher = Matcher::new(a, b);
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    for (ai, bj, size) in matcher.matching_blocks() {
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
}
