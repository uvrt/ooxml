//! Owns: automatic hyphenation's soft hyphens -- one at every point Word may break a word
//! (docx2svg's `linebreak.auto_hyphens` and `_token_hyphens`): words are runs of letters,
//! their language their first letter's run's; the shortest fragments, compounds, French
//! apostrophes, `w:noProof` and `w:doNotHyphenateCaps` as measured.
//!
//! Must not: decide whether a line takes a hyphen (that is [`crate::breaks`]).
//!
//! Python's `str.isalpha` and `str.isdigit` are read as Rust's `char::is_alphabetic` and
//! `char::is_numeric`; they differ for combining vowel signs, letter numbers and
//! fractions, which no measured hyphenated text holds.

use crate::hyphenate;
use crate::pieces::{Piece, PieceKind};
use crate::units::Units;
use std::collections::BTreeMap;

/// Per run: its language, `w:noProof`, and the width of its hyphen (`None`: unmeasurable).
pub(crate) type RunInfo = BTreeMap<i64, (Option<String>, bool, Option<Units>)>;

const JOINERS: [char; 4] = ['-', '\u{2011}', '\u{2013}', '\u{2014}'];
const FRENCH_APOSTROPHE: char = '\u{2019}';

fn is_word_piece(piece: &Piece) -> bool {
    matches!(piece.kind, PieceKind::Glyph | PieceKind::SoftHyphen) && piece.source >= 0
}

/// `items` with an automatic soft hyphen before every piece a word may break before.
pub(crate) fn auto_hyphens(items: Vec<Piece>, runs: &RunInfo, mode15: bool, no_caps: bool) -> Vec<Piece> {
    let right_min = if mode15 { 3 } else { 2 };
    let mut insert: BTreeMap<usize, Piece> = BTreeMap::new();
    let n = items.len();
    let mut i = 0;
    while i < n {
        if !items.get(i).is_some_and(is_word_piece) {
            i += 1;
            continue;
        }
        let mut j = i;
        while items.get(j).is_some_and(is_word_piece) {
            j += 1;
        }
        let token = Token {
            items: &items,
            runs,
            mode15,
            no_caps,
            right_min,
        };
        token.hyphens(i, j, &mut insert);
        i = j;
    }
    if insert.is_empty() {
        return items;
    }
    let mut out = Vec::with_capacity(items.len() + insert.len());
    for (index, piece) in items.into_iter().enumerate() {
        if let Some(hyphen) = insert.remove(&index) {
            out.push(hyphen);
        }
        out.push(piece);
    }
    out
}

struct Token<'a> {
    items: &'a [Piece],
    runs: &'a RunInfo,
    mode15: bool,
    no_caps: bool,
    right_min: usize,
}

impl Token<'_> {
    fn piece(&self, k: usize) -> Option<&Piece> {
        self.items.get(k)
    }

    fn glyph_char(&self, k: usize) -> Option<char> {
        self.piece(k).filter(|p| p.kind == PieceKind::Glyph).map(|p| p.ch)
    }

    /// The automatic hyphens of `items[start..end]`, into `insert`.
    fn hyphens(&self, start: usize, end: usize, insert: &mut BTreeMap<usize, Piece>) {
        let first = (start..end).find(|k| self.glyph_char(*k).is_some_and(char::is_alphabetic));
        let language = first.and_then(|k| self.piece(k)).and_then(|p| self.runs.get(&p.run));
        let french = language
            .and_then(|l| l.0.as_deref())
            .is_some_and(|l| l.to_lowercase().starts_with("fr"));
        let joiner = |c: char| JOINERS.contains(&c) || (french && c == FRENCH_APOSTROPHE);
        let letter = |k: usize| {
            self.piece(k)
                .is_some_and(|p| p.ch.is_alphabetic() || (french && p.ch == '\''))
        };
        let mut segments: Vec<Vec<usize>> = vec![Vec::new()];
        for k in start..end {
            if self.glyph_char(k).is_some_and(joiner) {
                segments.push(Vec::new());
            } else if let Some(last) = segments.last_mut() {
                last.push(k);
            }
        }
        let count = if self.mode15 { segments.len() } else { 1 };
        for segment in segments.iter().take(count) {
            let mut words: Vec<Vec<usize>> = vec![Vec::new()];
            for &k in segment {
                if self.piece(k).is_some_and(|p| p.kind == PieceKind::SoftHyphen) || letter(k) {
                    if let Some(last) = words.last_mut() {
                        last.push(k);
                    }
                } else {
                    words.push(vec![k]);
                    words.push(Vec::new());
                }
            }
            for index in 0..words.len() {
                self.word(&words, index, &letter, insert);
            }
        }
    }

    fn word(
        &self,
        words: &[Vec<usize>],
        index: usize,
        letter: &dyn Fn(usize) -> bool,
        insert: &mut BTreeMap<usize, Piece>,
    ) {
        let Some(word) = words.get(index) else { return };
        let mut letters: Vec<usize> = word
            .iter()
            .copied()
            .filter(|k| self.piece(*k).is_some_and(|p| p.kind == PieceKind::Glyph) && letter(*k))
            .collect();
        let Some(&first) = letters.first() else { return };
        let before = index.checked_sub(1).and_then(|b| words.get(b)).and_then(|w| w.first());
        let after = words.get(index + 1).and_then(|w| w.first());
        if before
            .into_iter()
            .chain(after)
            .any(|k| self.piece(*k).is_some_and(|p| p.ch.is_numeric()))
        {
            return;
        }
        let Some(info) = self.piece(first).and_then(|p| self.runs.get(&p.run)) else {
            return;
        };
        if info.2.is_none() {
            return;
        }
        let mut proof_end = None;
        for (count, &k) in letters.iter().enumerate() {
            let run = self.piece(k).map_or(-1, |p| p.run);
            let no_proof = self.runs.get(&run).map_or(info.1, |r| r.1);
            if no_proof && (count == 0 || !self.mode15) {
                proof_end = self.piece(k).map(|p| p.source);
                letters.truncate(count);
                break;
            }
        }
        let text: String = letters.iter().filter_map(|k| self.piece(*k)).map(|p| p.ch).collect();
        if text.is_empty() || (self.no_caps && text.to_uppercase() == text && text.to_lowercase() != text) {
            return;
        }
        let mut points = hyphenate::points(&text, info.0.as_deref(), self.right_min, self.mode15);
        if !self.mode15 {
            let soft = |k: usize| {
                k.checked_sub(1)
                    .and_then(|b| self.piece(b))
                    .is_some_and(|p| p.kind == PieceKind::SoftHyphen)
            };
            if let Some(first_soft) = letters.iter().enumerate().position(|(count, k)| count > 0 && soft(*k)) {
                points.retain(|p| *p < first_soft);
            }
        }
        for p in points {
            let (Some(&after), Some(&last)) = (letters.get(p), p.checked_sub(1).and_then(|q| letters.get(q))) else {
                continue;
            };
            if after
                .checked_sub(1)
                .and_then(|b| self.piece(b))
                .is_some_and(|q| q.kind == PieceKind::SoftHyphen)
            {
                continue;
            }
            let run = self.piece(last).map_or(-1, |q| q.run);
            let width = self.runs.get(&run).map_or(info.2, |r| r.2);
            if let (Some(width), Some(at)) = (width, self.piece(after)) {
                insert.insert(after, Piece::auto_hyphen(at.source, width, run, proof_end));
            }
        }
    }
}
