//! Owns: the places a text occurs for `word_format`'s `find` (docx-agent's `Document.find`:
//! each paragraph's current text alone, matches left to right; in the body, or in the
//! paragraphs a `within` address holds) as ranges with their text and context
//! ([`occurrences`]), and the text a range address holds ([`range_text`]).
//!
//! Must not: change anything.

use crate::find::{matches, Literal};
use crate::text_range::{range, range_id, Span};
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::address::{scope_paragraphs, RangeText};
use std::collections::BTreeSet;

/// A place a text occurs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    /// Its range id (`p:A@4:9`).
    pub id: String,
    /// The text there.
    pub text: String,
    /// The text with some of what surrounds it (`TextRange.context`).
    pub context: String,
}

/// Every place `text` occurs, in document order: in the body, or within the paragraphs
/// `within` holds; `case` false ignores case, `collapse` reads any white space as one.
///
/// # Errors
/// `within`'s refusal.
pub fn occurrences(
    doc: &WordDoc,
    text: &str,
    within: Option<&str>,
    case: bool,
    collapse: bool,
) -> Result<Vec<Occurrence>> {
    let docx = doc.docx()?;
    let (part, scope) = match within {
        Some(address) => {
            let (story, indexes) = scope_paragraphs(&docx, address).map_err(EditError::Address)?;
            let ids: BTreeSet<String> = indexes
                .iter()
                .filter_map(|k| story.ids.paragraphs().get(*k).map(|p| p.id))
                .collect();
            (story.part.clone(), Some(ids))
        }
        None => (docx.main.clone(), None),
    };
    let pattern = Literal::new(text, case, collapse, "")?;
    let mut out = Vec::new();
    for (start_id, start, end_id, end, _) in matches(&docx, &pattern, false)? {
        let found = RangeText {
            start_id,
            start,
            end_id,
            end,
        };
        let Ok(span) = Span::of(doc, &docx, &found) else {
            continue;
        };
        if span.part != part || scope.as_ref().is_some_and(|ids| !ids.contains(&found.start_id)) {
            continue;
        }
        out.push(Occurrence {
            id: range_id(&found),
            text: span.text(),
            context: span.context(20),
        });
    }
    Ok(out)
}

/// The text the range `address` names (a range, a paragraph, a cell...).
///
/// # Errors
/// The address's refusal.
pub fn range_text(doc: &WordDoc, address: &str) -> Result<String> {
    let docx = doc.docx()?;
    let found = range(doc, &docx, address)?;
    Ok(Span::of(doc, &docx, &found)?.text())
}
