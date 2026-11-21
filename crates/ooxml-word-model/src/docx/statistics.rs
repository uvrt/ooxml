//! Owns: the body's text statistics as docx-agent's `text_statistics` counts them for
//! `app.xml` -- words, characters with and without spaces, and paragraphs with text -- in
//! the final view (each paragraph's current text, [`super::PartTexts`]): words are runs of
//! characters that are not Python white space, characters leave white space out, the
//! spaces counted with them are spaces, no-break spaces and tabs, and a paragraph counts
//! when its text is not white space only.
//!
//! Must not: lay the document out (pages and lines are the layout's) or write anything.

use super::Docx;
use ooxml_xml::view::py_space;

/// The body's text statistics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextCounts {
    /// Words.
    pub words: u64,
    /// Characters, white space left out.
    pub characters: u64,
    /// Characters with the spaces, no-break spaces and tabs between them.
    pub characters_with_spaces: u64,
    /// Paragraphs with text.
    pub paragraphs: u64,
}

/// The body's text statistics (all zero without a body).
#[must_use]
pub fn text_counts(docx: &Docx) -> TextCounts {
    let mut out = TextCounts::default();
    let Some(body) = docx.body() else {
        return out;
    };
    let mut spaces = 0u64;
    for text in body.texts().iter() {
        if text.chars().all(py_space) {
            continue;
        }
        out.paragraphs += 1;
        out.words += text.split(py_space).filter(|w| !w.is_empty()).count() as u64;
        out.characters += text.chars().filter(|c| !py_space(*c)).count() as u64;
        spaces += text.chars().filter(|c| matches!(c, ' ' | '\u{a0}' | '\t')).count() as u64;
    }
    out.characters_with_spaces = out.characters + spaces;
    out
}
