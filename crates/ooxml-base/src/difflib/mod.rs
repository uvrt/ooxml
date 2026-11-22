//! Owns: Python's `difflib.SequenceMatcher(None, a, b, autojunk=False)`'s answers
//! ([`opcodes`]: the same longest matches, found in the same order, joined the same way, by
//! `matcher`'s suffix automaton and bounded there for long unlike texts; [`ratio`]), and the folding docx-agent and pptx-agent diff text by ([`coarse`]:
//! characters, short coincidental matches folded into the change around them; [`folded`]
//! for the callers' own rules).
//!
//! Must not: know what the sequences are (text, paragraphs, tokens).

use std::hash::Hash;

use matcher::matching_blocks;

mod matcher;

/// What an opcode does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    /// `a[i1..i2]` is `b[j1..j2]`.
    Equal,
    /// Anything else (replace, delete, insert; the foldings do not tell them apart).
    Change,
}

/// An opcode: the tag and the ranges `a[i1..i2]`, `b[j1..j2]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opcode {
    /// What it does.
    pub tag: Tag,
    /// Where it starts in `a`.
    pub i1: usize,
    /// Where it ends in `a`.
    pub i2: usize,
    /// Where it starts in `b`.
    pub j1: usize,
    /// Where it ends in `b`.
    pub j2: usize,
}

/// `difflib.SequenceMatcher(None, a, b, autojunk=False).ratio()`: twice the characters
/// the matching blocks hold over both lengths (1 for two empty sequences); past
/// `matcher::WORK`, the common start and end only.
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
/// and insert all [`Tag::Change`]; past `matcher::WORK`, the common start and end kept
/// and one change between.
pub fn opcodes<T: Eq + Hash>(a: &[T], b: &[T]) -> Vec<Opcode> {
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
pub fn folded(
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
pub fn coarse(old: &[char], new: &[char]) -> Vec<Opcode> {
    folded(&opcodes(old, new), |op| op.i2 - op.i1 < MIN_KEPT, |i| i, |j| j)
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
    fn paragraphs_as_difflib() {
        // difflib.SequenceMatcher(None, ["a", "b", "c"], ["a", "x", "c", "d"], autojunk=False)
        let (a, b) = (["a", "b", "c"], ["a", "x", "c", "d"]);
        assert_eq!(
            ops(opcodes(&a, &b)),
            vec![
                (true, 0, 1, 0, 1),
                (false, 1, 2, 1, 2),
                (true, 2, 3, 2, 3),
                (false, 3, 3, 3, 4)
            ]
        );
    }
}
