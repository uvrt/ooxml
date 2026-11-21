//! Owns: what a page's summary depends on beyond its laid page and the flow items it
//! draws ([`page_contexts`]), digested per page: its `PageInfo` (its number, section,
//! header kind, whether it is blank), the page count (`NUMPAGES`) and its section's
//! (`SECTIONPAGES`), its section's properties (its size, margins, borders, columns and
//! the headers and footers it shows), the texts its header's and footer's `STYLEREF`s
//! show on it, and the line numbers drawn beside its lines. Two pages with the same laid
//! page, the same items and the same context paint alike; the body's field values are in
//! its items already (a paragraph whose field shows another value is a new one).
//!
//! Must not: paint, or digest what is not the page's own (one page more changes only the
//! pages whose counts it changes, not every page's).

use ooxml_base::{Digest, DigestKey};
use ooxml_word_layout::fields::page_counts;
use ooxml_word_layout::DocumentLayout;
use ooxml_word_model::Model;
use std::collections::BTreeMap;

/// Each page's context (0-based, blank pages counted), digested under `key`.
pub(crate) fn page_contexts(
    layout: &DocumentLayout,
    model: &mut Model,
    locale: &ooxml_base::Locale,
    key: DigestKey,
) -> Vec<Digest> {
    let sections: Vec<Digest> = layout
        .document
        .sections
        .iter()
        .map(|s| Digest::of(key, format!("{:?}", s.properties).as_bytes()))
        .collect();
    // The `STYLEREF` texts each page's stories show, by page, in their stories' order.
    let mut references: BTreeMap<usize, Vec<(&str, String)>> = BTreeMap::new();
    for ((page, id), texts) in layout.story_texts(locale) {
        references.entry(*page).or_default().push((&**id, format!("{texts:?}")));
    }
    for texts in references.values_mut() {
        texts.sort_unstable();
    }
    let numbers = layout.line_numbers(model);
    layout
        .pagination
        .info
        .iter()
        .enumerate()
        .map(|(page, info)| {
            let counts = page_counts(layout, info.section);
            let section = sections.get(info.section).copied().unwrap_or_default().0;
            let referenced = references.get(&page).map(Vec::as_slice).unwrap_or_default();
            Digest::of(
                key,
                format!("{info:?}|{counts:?}|{section:?}|{referenced:?}|{:?}", numbers.on(page)).as_bytes(),
            )
        })
        .collect()
}
