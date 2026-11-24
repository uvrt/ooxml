//! Owns: `check`'s facts read from the document alone (docx-agent's `layout_facts.py`
//! functions that need no layout): blank paragraphs used as space ([`empty_paragraphs`]),
//! list levels whose labels show a level nothing uses ([`numbering_skips_level`]), tables off
//! their grid or ragged ([`table_grid_mismatch`]), the cells Word's Accept All removes
//! with their text ([`accept_loses_text`]), paragraphs whose direct formatting sets them
//! apart from their styles ([`direct_formatting`]), the styles in use that a list does
//! not allow ([`style_list`]), the file's text a pending deletion this session made takes
//! out of the final view ([`accept_loses_original`]), the rows the session wrote unlike the
//! rows around them ([`row_text_format`]), spacing around punctuation the text's language
//! does not want ([`punctuation_spacing`]) and text in another language than its tag
//! ([`text_language`]) -- each as typed entries in document order, with what fixes them.
//!
//! Must not: lay out, filter by pages or ranges, cap or word a reply (the tool layer does),
//! or change the document.

mod accept;
mod direct;
mod empty;
mod grid;
mod language;
mod numbering;
mod original;
mod proofing;
mod rows;
mod scratch;
mod shapes;
mod spacing;
mod styles;

pub use accept::{accept_loses_text, LostCell};
pub use direct::{direct_formatting, DirectFormatting, FormatValue};
pub use empty::{empty_paragraphs, EmptyRun};
pub(crate) use grid::{accepted_weight, AcceptedShapes};
pub use grid::{ragged_rows, table_grid_mismatch, GridFix, GridMismatch, OffGridRow, Ragged, RaggedRow};
pub use language::{
    detect, text_language, words_of, Detected, Detections, DocumentLanguage, Dominant, LanguageScope, Mistagged, Retag,
    TextLanguage, LANGUAGES, MIN_CONFIDENCE, MIN_WORDS,
};
pub use numbering::{numbering_skips_level, SkippedLevel};
pub use ooxml_word_model::typography::SpotKind;
pub use original::{accept_loses_original, LostOriginal};
pub use rows::row_text_format;
pub use spacing::{
    punctuation_spacing, LanguageMismatch, ReplaceCall, SetLanguage, Spacing, SpacingMark, SpacingProblem,
};
pub use styles::{style_list, StyleList, StyleUse};

use crate::names::W;
use ooxml_word_model::docx::Docx;
use ooxml_xml::Element;
use std::collections::HashMap;

/// Whether `c` is white space as Python's `str.strip` and `str.split` take it.
pub(crate) fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Each style's name by its id (its id where it has no name).
fn style_names(docx: &Docx) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for style in docx.styles() {
        if let Some(id) = style.attribute(Some(W), "styleId") {
            let name = style_name(style).unwrap_or_else(|| id.to_owned());
            out.insert(id.to_owned(), name);
        }
    }
    out
}

/// A style's `w:name` (`None` where it has none or an empty one).
fn style_name(style: &Element) -> Option<String> {
    style
        .find(Some(W), "name")
        .and_then(|n| n.attribute(Some(W), "val"))
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
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
