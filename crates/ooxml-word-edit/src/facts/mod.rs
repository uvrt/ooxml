//! Owns: `check`'s facts read from the document alone (docx-agent's `layout_facts.py`
//! functions that need no layout): blank paragraphs used as space ([`empty_paragraphs`]),
//! list levels whose labels show a level nothing uses ([`numbering_skips_level`]), tables off
//! their grid or ragged ([`table_grid_mismatch`]) and the cells Word's Accept All removes
//! with their text ([`accept_loses_text`]), spacing around punctuation the text's language
//! does not want ([`punctuation_spacing`]) and text in another language than its tag
//! ([`text_language`]) -- each as typed entries in document order, with what fixes them.
//!
//! Must not: lay out, filter by pages or ranges, cap or word a reply (the tool layer does),
//! or change the document.

mod accept;
mod empty;
mod grid;
mod language;
mod numbering;
mod proofing;
mod scratch;
mod spacing;

pub use accept::{accept_loses_text, LostCell};
pub use empty::{empty_paragraphs, EmptyRun};
pub use grid::{table_grid_mismatch, GridFix, GridMismatch, OffGridRow, RaggedRow};
pub use language::{
    detect, text_language, words_of, Detected, DocumentLanguage, Dominant, LanguageScope, Mistagged, Retag,
    TextLanguage, LANGUAGES, MIN_CONFIDENCE, MIN_WORDS,
};
pub use numbering::{numbering_skips_level, SkippedLevel};
pub use ooxml_word_model::typography::SpotKind;
pub use spacing::{
    punctuation_spacing, LanguageMismatch, ReplaceCall, SetLanguage, Spacing, SpacingMark, SpacingProblem,
};

/// Whether `c` is white space as Python's `str.strip` and `str.split` take it.
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Whether `c` is a letter of a word as Python's `re` reads `[^\W\d_]` in a `str` pattern:
/// a word character (`str.isalnum`) that is not a decimal digit or `_`.
///
/// Rust's predicates stand in for Python's here: `char::is_alphabetic` also takes combining
/// vowel signs (Indic and Arabic scripts), and non-ASCII decimal digits count as letters.
/// Neither shows in the languages `text_language` detects.
fn py_letter(c: char) -> bool {
    (c.is_alphabetic() || c.is_numeric()) && !c.is_ascii_digit()
}
