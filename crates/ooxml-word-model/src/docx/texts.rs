//! Owns: [`PartTexts`], each paragraph's text in the current view of one story part's
//! version ([`super::text::paragraph_text`], `View::Current`), in the order of the part's
//! paragraphs (`PartIds::paragraphs`), read once per version and kept with the part: a later
//! version takes the text of every paragraph it shares with an earlier one (the same
//! element, by pointer) and walks only the paragraphs that changed. A find reads these
//! texts instead of walking every paragraph again (`find_text`, `replace_text`, the edit
//! core's finds).
//!
//! Must not: keep anything a part's tree does not determine, or a text of any other view.

use crate::docx::text::{paragraph_text, View};
use crate::ids::PartIds;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// One paragraph: its element (held, so its pointer is never another's) and its text.
#[derive(Debug, Clone)]
struct Line {
    element: Arc<Element>,
    text: Arc<str>,
}

/// A story part's paragraph texts at one version, in document order.
#[derive(Debug, Default)]
pub struct PartTexts {
    lines: Vec<Line>,
}

impl PartTexts {
    /// The texts of the paragraphs `ids` lists; `earlier`, the texts of an earlier version
    /// of the part, whose paragraphs this version shares are taken as they are.
    #[must_use]
    pub fn read(ids: &PartIds, earlier: Option<&PartTexts>) -> PartTexts {
        let paragraphs = ids.paragraphs();
        let old: &[Line] = earlier.map_or(&[], |e| &e.lines);
        // A paragraph changed in place keeps every other one's place; one inserted or
        // removed moves the later ones by the difference: both are tried first. Paragraphs
        // moved further are looked up by pointer, from the second one not found so (an edit
        // of one paragraph makes one new element, found nowhere).
        let shift = paragraphs.len().abs_diff(old.len());
        let grown = paragraphs.len() >= old.len();
        let mut by_pointer: Option<HashMap<*const Element, &Line>> = None;
        let mut missed = 0_usize;
        let mut lines = Vec::with_capacity(paragraphs.len());
        for (k, element) in paragraphs.elements().enumerate() {
            let same = |line: &&Line| Arc::ptr_eq(&line.element, element);
            let shifted = if grown {
                k.checked_sub(shift)
            } else {
                k.checked_add(shift)
            };
            let found = old
                .get(k)
                .filter(same)
                .or_else(|| shifted.and_then(|s| old.get(s)).filter(same))
                .map(|line| Arc::clone(&line.text))
                .or_else(|| {
                    missed += 1;
                    if missed < 2 {
                        return None;
                    }
                    let map =
                        by_pointer.get_or_insert_with(|| old.iter().map(|l| (Arc::as_ptr(&l.element), l)).collect());
                    map.get(&Arc::as_ptr(element)).map(|l| Arc::clone(&l.text))
                });
            let text = found.unwrap_or_else(|| Arc::from(paragraph_text(element, View::Current)));
            lines.push(Line {
                element: Arc::clone(element),
                text,
            });
        }
        PartTexts { lines }
    }

    /// The text of the `k`-th paragraph (as `PartIds::paragraphs` numbers them).
    #[must_use]
    pub fn get(&self, k: usize) -> Option<&str> {
        self.lines.get(k).map(|line| &*line.text)
    }

    /// How many paragraphs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// No paragraphs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Every paragraph's text, in document order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(|line| &*line.text)
    }
}
