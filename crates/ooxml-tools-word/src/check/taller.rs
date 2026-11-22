//! Owns: how tall the blocks a changing call keeps together are drawn in the version before
//! its change, where the document's last check laid that very version out (docx-agent's
//! `_laid_before` and `_taller_than_page`, read by `word_format` `keep_together`): the
//! targets taller than the room a page gives the body.
//!
//! Must not: lay out (only what the last check laid out is read), or change the document.

use super::layout::Painted;
use super::reflow::LastCheck;
use ooxml_word_model::docx::Docx;
use ooxml_word_pages::layout_facts::Facts;
use ooxml_word_pages::summarised::Summarised;
use std::collections::BTreeMap;

/// Of `kept` -- each target with the ids of its paragraphs -- those the version `version`
/// (read as `docx`) draws taller than a page's body holds, with their height and the least
/// room a page they are on gives the body, in points rounded to a tenth (docx-agent's
/// `_taller_than_page`: taller by more than half a point). Empty where `last` did not lay
/// `version` out itself.
#[must_use]
pub(crate) fn taller_than_page(
    last: &LastCheck,
    version: u64,
    docx: &Docx,
    kept: &[(String, Vec<String>)],
) -> BTreeMap<String, (f64, f64)> {
    let mut out = BTreeMap::new();
    let Some(Summarised::Laid { version: laid, engine }) = last
        .laid()
        .filter(|seen| seen.version == Some(version))
        .map(|seen| seen.pages)
    else {
        return out;
    };
    let painted = Painted {
        engine: &engine,
        version: std::sync::Arc::clone(&laid),
    };
    let facts = Facts::new(&laid, docx, None, &painted);
    for (address, ids) in kept {
        let extents = facts.extent(ids, None);
        if extents.is_empty() {
            continue;
        }
        let height = tenth(extents.iter().map(|e| e.bottom - e.top).sum());
        let room = tenth(
            extents
                .iter()
                .map(|e| e.area.1 - e.area.0)
                .fold(f64::INFINITY, f64::min),
        );
        if height > room + 0.5 {
            out.insert(address.clone(), (height, room));
        }
    }
    out
}

/// `value` rounded to a tenth, as Python's `round(value, 1)`.
fn tenth(value: f64) -> f64 {
    ooxml_word_pages::layout_facts::round(value, 1)
}
