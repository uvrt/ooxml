//! Owns: where a find matches a version's text, as docx-agent's `_matches` finds it --
//! every story's paragraphs in order, each paragraph's current text alone or (across
//! paragraphs) a story's paragraphs joined by `\n`, matches left to right and not
//! overlapping, empty ones left out -- and the [`Pattern`] a find is made with: plain text
//! ([`Literal`]: case-sensitive or not, white space collapsed or not, as `re.escape`d text
//! matches) or one the caller supplies (a Python regular expression, matched by the tool
//! layer's translation: this crate holds no regex engine).
//!
//! Must not: change anything, or answer a pattern differently from Python's `re`.

use crate::text::diff::python_space;
use crate::text::paragraph_text;
use crate::work::Work;
use crate::{EditError, Result};
use ooxml_word_model::docx::Docx;
use std::fmt::Debug;

/// A find: where it matches a text, and what replaces each match.
pub trait Pattern: Debug + Send + Sync {
    /// Each match's start and end (in characters, left to right, not overlapping, none
    /// empty) and its replacement (a regular expression's template expanded).
    ///
    /// # Errors
    /// A message when the pattern cannot be matched as Python would.
    fn matches(&self, text: &str) -> std::result::Result<Vec<(usize, usize, String)>, String>;
}

/// Plain text, as `re.escape(text)` (or, with white space collapsed, its words joined by
/// `\s+`) matches it, replaced by `with`.
#[derive(Debug, Clone)]
pub struct Literal {
    words: Vec<Vec<char>>,
    case: bool,
    with: String,
}

impl Literal {
    /// The find of `text` (`case`: case matters; `collapse`: any run of white space
    /// matches any other), replaced by `with`.
    ///
    /// # Errors
    /// `nothing to find` for empty text (or, collapsed, text of white space only).
    pub fn new(text: &str, case: bool, collapse: bool, with: &str) -> Result<Literal> {
        let words: Vec<Vec<char>> = if collapse {
            split_python(text).into_iter().map(|w| w.chars().collect()).collect()
        } else {
            vec![text.chars().collect()]
        };
        if text.is_empty() || words.is_empty() || words.iter().all(Vec::is_empty) {
            return Err(EditError::Refused("nothing to find".to_owned()));
        }
        Ok(Literal {
            words,
            case,
            with: with.to_owned(),
        })
    }

    fn same(&self, a: char, b: char) -> bool {
        let lower = |c: char| {
            let mut l = c.to_lowercase();
            match (l.next(), l.next()) {
                (Some(one), None) => one,
                _ => c,
            }
        };
        a == b || (!self.case && lower(a) == lower(b))
    }

    /// The end of a match starting at `k`, if one does.
    fn match_at(&self, text: &[char], k: usize) -> Option<usize> {
        let mut at = k;
        for (n, word) in self.words.iter().enumerate() {
            if n > 0 {
                let spaces = text.get(at..)?.iter().take_while(|c| python_space(**c)).count();
                if spaces == 0 {
                    return None;
                }
                at += spaces;
            }
            let here = text.get(at..at + word.len())?;
            if !here.iter().zip(word).all(|(a, b)| self.same(*a, *b)) {
                return None;
            }
            at += word.len();
        }
        Some(at)
    }
}

/// Python's `str.split()`: the words between runs of white space.
fn split_python(text: &str) -> Vec<String> {
    text.split(python_space)
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

impl Pattern for Literal {
    fn matches(&self, text: &str) -> std::result::Result<Vec<(usize, usize, String)>, String> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut k = 0;
        while k < chars.len() {
            match self.match_at(&chars, k) {
                Some(end) if end > k => {
                    out.push((k, end, self.with.clone()));
                    k = end;
                }
                _ => k += 1,
            }
        }
        Ok(out)
    }
}

/// A match: its first paragraph's id and offset, its last's, and its replacement.
pub(crate) type Found = (String, usize, String, usize, String);

/// Every match of `pattern` in `docx`'s stories, in document order (`across`: in each
/// story's paragraphs joined by `\n`).
pub(crate) fn matches(docx: &Docx, pattern: &dyn Pattern, across: bool) -> Result<Vec<Found>> {
    let refuse = |e: String| EditError::Refused(e);
    let mut out = Vec::new();
    for story in &docx.parts {
        let paragraphs: Vec<(String, String)> = story
            .ids
            .paragraphs()
            .iter()
            .map(|p| {
                let work = Work::of(&p.element, Vec::new());
                (p.id, paragraph_text(&work, work.root()))
            })
            .collect();
        if !across {
            for (id, text) in &paragraphs {
                for (a, b, with) in pattern.matches(text).map_err(refuse)? {
                    out.push((id.clone(), a, id.clone(), b, with));
                }
            }
            continue;
        }
        let mut starts = Vec::with_capacity(paragraphs.len());
        let mut joined = String::new();
        let mut position = 0;
        for (k, (_, text)) in paragraphs.iter().enumerate() {
            if k > 0 {
                joined.push('\n');
            }
            starts.push(position);
            position += text.chars().count() + 1;
            joined.push_str(text);
        }
        // the paragraph an offset is in; an end at a paragraph's start is offset 0 there
        let locate = |offset: usize| starts.partition_point(|s| *s <= offset).saturating_sub(1);
        for (a, b, with) in pattern.matches(&joined).map_err(refuse)? {
            let (p, q) = (locate(a), locate(b));
            let (Some((first, _)), Some((last, _)), Some(sp), Some(sq)) =
                (paragraphs.get(p), paragraphs.get(q), starts.get(p), starts.get(q))
            else {
                continue;
            };
            out.push((first.clone(), a - sp, last.clone(), b - sq, with));
        }
    }
    Ok(out)
}
