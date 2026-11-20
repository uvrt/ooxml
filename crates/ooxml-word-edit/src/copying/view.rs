//! Owns: a copied block made the source's current view, as docx-agent's importer makes it
//! (`_current_view`, and `bring`'s first steps): insertions kept, deletions dropped, the
//! records of changes dropped, a paragraph's section break and a control's data binding
//! left out; and what the copy holds that this port leaves to the host (lists,
//! relationships, notes, comments, bookmarks, drawings, controls).
//!
//! Must not: read the documents; it works on the copy alone.

use crate::names::{O, R, W, WP};
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

/// `element` (a copy, detached or placed) as the source shows it now.
pub(crate) fn current_view(work: &mut Work, element: Id) {
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
    for section in work.iter(element, W, &["sectPr"]) {
        if work.parent(section).is_some_and(|p| work.is(p, W, "pPr")) {
            work.remove(section);
        }
    }
    for binding in work.iter(element, W, &["dataBinding"]) {
        work.remove(binding);
    }
}

/// What in the copies this port leaves to the host, if anything: the first kind found.
pub(crate) fn left_to_host(work: &Work, elements: &[Id]) -> Option<&'static str> {
    let kinds: [(&str, &[&str], &'static str); 6] = [
        (W, &["numPr"], "a copied list"),
        (W, &["footnoteReference", "endnoteReference"], "a copied note"),
        (
            W,
            &["commentRangeStart", "commentRangeEnd", "commentReference"],
            "a copied comment",
        ),
        (W, &["bookmarkStart", "bookmarkEnd"], "a copied bookmark"),
        (W, &["sdt"], "a copied content control"),
        (WP, &["docPr"], "a copied drawing"),
    ];
    for &element in elements {
        for (namespace, locals, what) in kinds {
            if work.is_any(element, namespace, locals) || !work.iter(element, namespace, locals).is_empty() {
                return Some(what);
            }
        }
        let related = work.descendants(element).into_iter().any(|node| {
            work.attributes(node).iter().any(|a| {
                let name = &a.name;
                name.namespace() == Some(R) || (name.namespace() == Some(O) && name.local() == "relid")
            })
        });
        if related {
            return Some("a copied relationship");
        }
    }
    None
}
