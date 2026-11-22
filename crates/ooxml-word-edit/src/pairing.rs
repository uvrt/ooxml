//! Owns: how a copy's parts correspond to its sample's, as docx-agent's `edit/compare.py`
//! pairs them for `word_compare_blocks` and `word_follow_sample` (its "Corresponding
//! parts"): a table's header rows (`w:tblHeader` read as Word reads it), rows by the
//! sample's structure -- its header rows one to one, body row *i* to the sample's body row
//! *i*, its last body row the pattern for any more -- and a cell's paragraphs by text, then
//! by style, in order, repeats of a pattern after it.
//!
//! Must not: read a document: callers hand it counts, header flags and each paragraph's
//! [`Signature`].

use crate::markdown::py_space;
use crate::text::diff::ratio;
use ooxml_word_model::docx::casefold;

/// Two paragraphs' normalised texts at least this alike (`difflib`'s ratio) match by text.
const TEXT_SIMILARITY: f64 = 0.8;

/// A paragraph as pairing reads it: its style's name and its text, case and white space
/// normalised ([`signature`]).
pub type Signature = (Option<String>, String);

/// `text` with Python's white space runs made one space, trimmed (`" ".join(text.split())`).
#[must_use]
pub fn collapse(text: &str) -> String {
    text.split(py_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A paragraph's signature: its style's name and its text, collapsed and case-folded.
#[must_use]
pub fn signature(style: Option<String>, text: &str) -> Signature {
    (style, casefold(&collapse(text)))
}

/// Whether a row's `w:tblHeader` (its `w:val`, `None` where it states none) makes it a
/// header row.
#[must_use]
pub fn is_header(value: Option<&str>) -> bool {
    let value = value.filter(|v| !v.is_empty()).unwrap_or("1");
    !matches!(value.to_lowercase().as_str(), "0" | "false" | "off")
}

/// Per row of a copy (`rows` of them), the sample's row it corresponds to (the module's
/// rule); the sample has `reference_rows` rows, the first `header` of them header rows.
#[must_use]
pub fn row_map(rows: usize, reference_rows: usize, header: usize) -> Vec<usize> {
    let body = reference_rows.saturating_sub(header);
    (0..rows)
        .map(|r| {
            if r < header || body == 0 {
                r.min(reference_rows.saturating_sub(1))
            } else {
                header + (r - header).min(body - 1)
            }
        })
        .collect()
}

/// A cell's paragraphs paired with its sample cell's, by index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Aligned {
    /// `(paragraph, sample's)` pairs.
    pub pairs: Vec<(usize, usize)>,
    /// The sample's paragraphs left unpaired.
    pub missing: Vec<usize>,
    /// The copy's paragraphs left unpaired.
    pub extra: Vec<usize>,
}

/// Whether two normalised texts match (the same, or at least [`TEXT_SIMILARITY`] alike:
/// `difflib`'s ratio).
#[must_use]
pub fn by_text(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    ratio(&a, &b) >= TEXT_SIMILARITY
}

/// The copy's paragraphs (`mine`) paired with the sample's (`theirs`), by their signatures
/// (docx-agent's `_align`): one each pair; else the longest alignment where a pair matching
/// by text outweighs any number matching by style alone, and -- where the counts are equal
/// and no paragraph matches another's text out of order -- position by position; a copy's
/// paragraph after an aligned one, in that one's style, pairs with the same sample
/// paragraph (a repeat of the pattern).
#[must_use]
pub fn align(mine: &[Signature], theirs: &[Signature]) -> Aligned {
    let (m, n) = (mine.len(), theirs.len());
    let zipped = || Aligned {
        pairs: (0..m.min(n)).map(|k| (k, k)).collect(),
        missing: Vec::new(),
        extra: Vec::new(),
    };
    if m == 1 && n == 1 {
        return zipped();
    }
    let text: Vec<Vec<bool>> = mine
        .iter()
        .map(|a| theirs.iter().map(|b| by_text(&a.1, &b.1)).collect())
        .collect();
    let matches_text = |i: usize, j: usize| text.get(i).and_then(|row| row.get(j)).copied().unwrap_or(false);
    let same_style = |i: usize, j: usize| mine.get(i).zip(theirs.get(j)).is_some_and(|(a, b)| a.0 == b.0);
    // a pair matching by text outweighs any number matching by style alone
    let weight = m + n + 1;
    let score = |i: usize, j: usize| weight * usize::from(matches_text(i, j)) + usize::from(same_style(i, j));
    let mut best = vec![vec![0usize; n + 1]; m + 1];
    let at = |best: &Vec<Vec<usize>>, i: usize, j: usize| best.get(i).and_then(|r| r.get(j)).copied().unwrap_or(0);
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            let s = score(i, j);
            let taken = if s > 0 { s + at(&best, i + 1, j + 1) } else { 0 };
            let value = at(&best, i + 1, j).max(at(&best, i, j + 1)).max(taken);
            if let Some(cell) = best.get_mut(i).and_then(|r| r.get_mut(j)) {
                *cell = value;
            }
        }
    }
    let mut aligned: Vec<(usize, usize)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < m && j < n {
        let s = score(i, j);
        if s > 0 && at(&best, i, j) == s + at(&best, i + 1, j + 1) {
            aligned.push((i, j));
            (i, j) = (i + 1, j + 1);
        } else if at(&best, i, j) == at(&best, i, j + 1) {
            j += 1;
        } else {
            i += 1;
        }
    }
    if m == n && !aligned.iter().any(|(i, j)| i != j && matches_text(*i, *j)) {
        return zipped();
    }
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut previous: Option<usize> = None;
    for i in 0..m {
        if let Some((_, j)) = aligned.iter().find(|(a, _)| *a == i) {
            previous = Some(*j);
            pairs.push((i, *j));
        } else if let Some(j) = previous.filter(|j| same_style(i, *j)) {
            pairs.push((i, j)); // a repeat of the pattern
        }
    }
    Aligned {
        missing: (0..n).filter(|j| !aligned.iter().any(|(_, b)| b == j)).collect(),
        extra: (0..m).filter(|i| !pairs.iter().any(|(a, _)| a == i)).collect(),
        pairs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(style: &str, text: &str) -> Signature {
        signature(Some(style.to_owned()), text)
    }

    #[test]
    fn rows_follow_the_samples_structure() {
        assert_eq!(row_map(5, 3, 1), vec![0, 1, 2, 2, 2]);
        assert_eq!(row_map(2, 1, 1), vec![0, 0]);
        assert!(is_header(None) && is_header(Some("true")) && !is_header(Some("off")));
    }

    #[test]
    fn paragraphs_pair_by_text_then_style_and_repeat_a_pattern() {
        let theirs = [sig("Title", "Total"), sig("Body", "Notes")];
        let mine = [sig("Body", "Notes"), sig("Body", "more"), sig("Title", "Total")];
        let found = align(&mine, &theirs);
        assert_eq!(found.pairs, vec![(0, 1), (1, 1)]);
        assert_eq!(found.missing, vec![0]);
        assert_eq!(found.extra, vec![2]);
        let same = align(&theirs, &theirs);
        assert_eq!(same.pairs, vec![(0, 0), (1, 1)]);
    }
}
