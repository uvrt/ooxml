//! Owns: the diffs set_text writes by -- Python's `difflib` opcodes and docx-agent's
//! [`coarse`] folding of them (`ooxml_base::difflib`, shared with the chart text edits) --
//! and docx-agent's word folding, [`by_words`] (a tracked replacement's stretches at word
//! boundaries); and [`ratio`], how alike two texts are.
//!
//! Must not: be used to find what a change touched (the change set says that); this is the
//! edit's own rule for where new characters go.

use ooxml_base::difflib::folded;
pub(crate) use ooxml_base::difflib::{coarse, opcodes, ratio, Opcode, Tag};

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
