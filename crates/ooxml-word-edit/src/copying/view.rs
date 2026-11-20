//! Owns: a copied block made the source's current view, as docx-agent's importer makes it
//! (`_current_view`, and `bring`'s first steps): insertions kept, deletions dropped, the
//! records of changes dropped, a paragraph's section break and a control's data binding
//! left out; and what the copy holds that this port leaves to the host (controls).
//!
//! Must not: read the documents; it works on the copy alone.

use crate::names::W;
use crate::work::{Id, Work};

/// The records of changes a copy drops wherever they are (`_current_view`).
const RECORDS: [&str; 15] = [
    "rPrChange",
    "pPrChange",
    "sectPrChange",
    "tblPrChange",
    "trPrChange",
    "tcPrChange",
    "tblGridChange",
    "numberingChange",
    "cellIns",
    "cellDel",
    "cellMerge",
    "moveFromRangeStart",
    "moveFromRangeEnd",
    "moveToRangeStart",
    "moveToRangeEnd",
];

/// `element` (a copied block, detached or placed) as the source shows it now, its section
/// break and data bindings left out (`bring`'s first steps).
pub(crate) fn current_view(work: &mut Work, element: Id) {
    records_view(work, element);
    for section in work.iter(element, W, &["sectPr"]) {
        if work.parent(section).is_some_and(|p| work.is(p, W, "pPr")) {
            work.remove(section);
        }
    }
    for binding in work.iter(element, W, &["dataBinding"]) {
        work.remove(binding);
    }
}

/// `element` with insertions kept, deletions dropped and the records of changes dropped
/// (`_current_view`: what a copied note's content is given too).
pub(crate) fn records_view(work: &mut Work, element: Id) {
    for node in work.iter(element, W, &["del", "moveFrom"]) {
        let Some(parent) = work.parent(node) else { continue };
        if work.is(parent, W, "trPr") && work.is(node, W, "del") {
            if let Some(row) = work.parent(parent).filter(|r| work.parent(*r).is_some()) {
                work.remove(row);
                continue;
            }
        }
        work.remove(node);
    }
    for node in work.iter(element, W, &["ins", "moveTo"]) {
        let Some(parent) = work.parent(node) else { continue };
        if work.is_any(parent, W, &["rPr", "trPr", "tcPr"]) {
            work.remove(node);
            continue;
        }
        for child in work.children(node).to_vec() {
            work.add_previous(node, child);
        }
        work.remove(node);
    }
    for record in RECORDS {
        for node in work.iter(element, W, &[record]) {
            if work.parent(node).is_some() {
                work.remove(node);
            }
        }
    }
}

/// What in the copies this port leaves to the host, if anything: the first kind found.
pub(crate) fn left_to_host(work: &Work, elements: &[Id]) -> Option<&'static str> {
    let kinds: [(&[&str], &'static str); 1] = [(&["sdt"], "a copied content control")];
    kinds
        .into_iter()
        .find(|(locals, _)| holds(work, elements, locals))
        .map(|(_, what)| what)
}

/// Whether any of `elements` is or holds a `w:` element named one of `locals`.
pub(crate) fn holds(work: &Work, elements: &[Id], locals: &[&str]) -> bool {
    elements
        .iter()
        .any(|&e| work.is_any(e, W, locals) || !work.iter(e, W, locals).is_empty())
}
