//! Owns: the tree helpers the review's rules are written with, over a [`Work`] tree, each
//! as docx-agent's review reads or writes the XML (`revisions/review.py`'s module
//! functions, ooxml-edit's `insert_in_order`, `edit/document.py`'s `_rehome_markers`):
//! what a record is ([`kind_of`]), ancestors, a paragraph's content (in-order insertion
//! and a mark's record are `crate::marks`', re-exported),
//! range markers kept when their paragraph goes, and what a review leaves empty
//! ([`prune`]).
//!
//! Must not: decide what a review accepts or rejects, or reach outside the tree it is given.

use crate::names::{M, R, W};
pub(crate) use crate::marks::{insert_in_order, mark_record};
use crate::work::{Id, Work};

/// Revision containers: what a run-level record wraps, and a mark's or row's own record.
pub(crate) const CONTAINERS: [&str; 4] = ["ins", "del", "moveFrom", "moveTo"];
/// Shown in the current view.
pub(crate) const INSERTED: [&str; 2] = ["ins", "moveTo"];
/// Hidden in the current view.
pub(crate) const DELETED: [&str; 2] = ["del", "moveFrom"];
/// A cell's own records.
pub(crate) const CELL: [&str; 3] = ["cellIns", "cellDel", "cellMerge"];
/// Property change records.
pub(crate) const PROPERTY_CHANGES: [&str; 8] = [
    "rPrChange",
    "pPrChange",
    "sectPrChange",
    "tblPrChange",
    "trPrChange",
    "tcPrChange",
    "tblGridChange",
    "numberingChange",
];
/// Every revision record's local name.
pub(crate) const RECORDS: [&str; 15] = [
    "ins",
    "del",
    "moveFrom",
    "moveTo",
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
];
/// Range markers: a start and its end share `w:id`.
const RANGE_PAIRS: [(&str, &str); 5] = [
    ("bookmarkStart", "bookmarkEnd"),
    ("commentRangeStart", "commentRangeEnd"),
    ("moveFromRangeStart", "moveFromRangeEnd"),
    ("moveToRangeStart", "moveToRangeEnd"),
    ("permStart", "permEnd"),
];
/// Blocks of a container.
pub(crate) const BLOCKS: [&str; 4] = ["p", "tbl", "sdt", "customXml"];

/// What a revision record is, by where it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordKind {
    /// A paragraph mark's record (`w:pPr/w:rPr/w:ins`).
    Mark,
    /// A row's (`w:trPr/w:ins`).
    Row,
    /// A content container around runs.
    Run,
    /// A cell's own record.
    Cell,
    /// A property change.
    Property,
    /// A run mark's or numbering's record: dropped as a record.
    Other,
}

/// What `element` is as a record (docx-agent's `_kind_of`).
pub(crate) fn kind_of(work: &Work, element: Id) -> RecordKind {
    if work.is_any(element, W, &CONTAINERS) {
        let parent = work.parent(element);
        let is = |id: Option<Id>, local: &str| id.is_some_and(|p| work.is(p, W, local));
        if is(parent, "rPr") && is(parent.and_then(|p| work.parent(p)), "pPr") {
            return RecordKind::Mark;
        }
        if is(parent, "trPr") {
            return RecordKind::Row;
        }
        if is(parent, "rPr") || is(parent, "numPr") {
            return RecordKind::Other;
        }
        return RecordKind::Run;
    }
    if work.is_any(element, W, &CELL) {
        return RecordKind::Cell;
    }
    if work.is_any(element, W, &PROPERTY_CHANGES) {
        return RecordKind::Property;
    }
    RecordKind::Other
}

/// Whether `element` is a revision record.
pub(crate) fn is_record(work: &Work, element: Id) -> bool {
    work.is_any(element, W, &RECORDS)
}

/// The nearest ancestor of `node` named `w:<local>`.
pub(crate) fn ancestor(work: &Work, node: Id, local: &str) -> Option<Id> {
    let mut at = work.parent(node);
    while let Some(p) = at {
        if work.is(p, W, local) {
            return Some(p);
        }
        at = work.parent(p);
    }
    None
}

/// Whether `node` is attached to the tree (the root, or under it).
pub(crate) fn attached(work: &Work, node: Id) -> bool {
    let mut at = node;
    while let Some(p) = work.parent(at) {
        at = p;
    }
    at == work.root()
}

/// The first child element of `id` at `path` (`w:` local names).
pub(crate) fn find_path(work: &Work, id: Id, path: &[&str]) -> Option<Id> {
    let mut at = id;
    for local in path {
        at = work.find(at, W, local)?;
    }
    Some(at)
}

/// The child elements of `id` named `w:<local>`.
pub(crate) fn find_all(work: &Work, id: Id, local: &str) -> Vec<Id> {
    work.children(id)
        .iter()
        .copied()
        .filter(|c| work.is(*c, W, local))
        .collect()
}

/// The `w:<local>` attribute.
pub(crate) fn attr<'w>(work: &'w Work, id: Id, local: &str) -> Option<&'w str> {
    work.get(id, Some(W), local)
}

/// A `w:` attribute's value as a whole number, when it is all ASCII digits (Python's
/// `isdigit()` then `int()`).
pub(crate) fn digits(work: &Work, id: Option<Id>, local: &str) -> Option<i64> {
    let value = attr(work, id?, local)?;
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

/// Python's `int()` of a `w:` attribute, `default` when it has none or is no number.
pub(crate) fn int(work: &Work, id: Option<Id>, local: &str, default: i64) -> i64 {
    id.and_then(|i| attr(work, i, local))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// A new `w:<local>` element, detached, with `w:` attributes.
pub(crate) fn make(work: &mut Work, local: &str, attributes: &[(&str, &str)]) -> Id {
    let element = work.new_element(work.qname(W, local, "w"));
    for (name, value) in attributes {
        let name = work.qname(W, name, "w");
        work.set(element, name, value);
    }
    element
}

/// Set the `w:<local>` attribute.
pub(crate) fn set_attr(work: &mut Work, id: Id, local: &str, value: &str) {
    let name = work.qname(W, local, "w");
    work.set(id, name, value);
}

/// Whether a paragraph holds anything but properties and range markers.
pub(crate) fn has_content(work: &Work, paragraph: Id) -> bool {
    work.descendants(paragraph)
        .into_iter()
        .skip(1)
        .any(|n| work.is_any(n, W, &["r", "fldSimple"]) || work.is(n, M, "oMath"))
}

/// The paragraph after `paragraph` in its container (none at a table or the end).
pub(crate) fn next_paragraph(work: &Work, paragraph: Id) -> Option<Id> {
    let mut following = work.next(paragraph);
    while let Some(node) = following {
        if work.is(node, W, "p") {
            return Some(node);
        }
        if work.is_any(node, W, &["tbl", "sdt", "customXml", "sectPr"]) {
            return None;
        }
        following = work.next(node);
    }
    None
}

/// The block after `node` in its container (a paragraph, table, content control,
/// `w:customXml`, or the body's `w:sectPr`), range markers passed over.
pub(crate) fn next_block(work: &Work, node: Id) -> Option<Id> {
    let mut following = work.next(node);
    while let Some(n) = following {
        if work.is_any(n, W, &BLOCKS) || work.is(n, W, "sectPr") {
            return Some(n);
        }
        following = work.next(n);
    }
    None
}

/// Move the range markers in `element` whose partner is outside it to the nearest
/// paragraph that stays, so no range is left with one end (`_rehome_markers`).
pub(crate) fn keep_markers(work: &mut Work, element: Id) {
    let mut inside: Vec<((String, String), Id)> = Vec::new();
    for node in work.descendants(element) {
        let Some(name) = work.name(node).filter(|n| n.namespace() == Some(W)) else {
            continue;
        };
        let local = name.local();
        if !RANGE_PAIRS.iter().any(|(s, e)| *s == local || *e == local) {
            continue;
        }
        let key = (local.to_owned(), attr(work, node, "id").unwrap_or("").to_owned());
        match inside.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = node,
            None => inside.push((key, node)),
        }
    }
    let lonely: Vec<Id> = inside
        .iter()
        .filter(|((local, id), _)| {
            let partner = RANGE_PAIRS
                .iter()
                .find_map(|(s, e)| (s == local).then_some(*e).or((e == local).then_some(*s)));
            !partner.is_some_and(|p| inside.iter().any(|((l, i), _)| l == p && i == id))
        })
        .map(|(_, node)| *node)
        .collect();
    if lonely.is_empty() {
        return;
    }
    let (previous, following) = neighbour_paragraphs(work, element);
    for node in lonely {
        work.set_tail(node, "");
        if let Some(following) = following {
            match work.find(following, W, "pPr") {
                Some(properties) => work.add_next(properties, node),
                None => work.insert(following, 0, node),
            }
        } else if let Some(previous) = previous {
            work.append(previous, node);
        }
    }
}

/// The paragraphs of the whole tree right before and right after those in `element`.
fn neighbour_paragraphs(work: &Work, element: Id) -> (Option<Id>, Option<Id>) {
    let paragraphs = work.iter(work.root(), W, &["p"]);
    let inside = work.iter(element, W, &["p"]);
    let positions: Vec<usize> = paragraphs
        .iter()
        .enumerate()
        .filter(|(_, p)| inside.contains(p))
        .map(|(k, _)| k)
        .collect();
    let (Some(&first), Some(&last)) = (positions.first(), positions.iter().max()) else {
        return (None, None);
    };
    let following = paragraphs
        .get(last + 1..)
        .and_then(|rest| rest.iter().copied().find(|p| !inside.contains(p)));
    let previous = first.checked_sub(1).and_then(|k| paragraphs.get(k)).copied();
    (previous, following)
}

/// Remove what a review emptied in `paragraph`: revision containers, hyperlinks, smart
/// tags and simple fields with nothing but properties (`_prune`); the hyperlinks'
/// relationship ids.
pub(crate) fn prune(work: &mut Work, paragraph: Id) -> Vec<String> {
    let mut released = Vec::new();
    let mut changed = true;
    let locals = [
        "r",
        "hyperlink",
        "ins",
        "del",
        "moveFrom",
        "moveTo",
        "smartTag",
        "fldSimple",
    ];
    while changed {
        changed = false;
        for node in work.iter(paragraph, W, &locals) {
            let Some(parent) = work.parent(node) else { continue };
            if work.is(node, W, "r") || work.is(parent, W, "rPr") {
                continue;
            }
            let kept = work
                .children(node)
                .iter()
                .any(|c| work.is_element(*c) && !work.is(*c, W, "rPr"));
            if !kept {
                if let Some(rid) = work.get(node, Some(R), "id").filter(|v| !v.is_empty()) {
                    released.push(rid.to_owned());
                }
                work.remove(node);
                changed = true;
            }
        }
    }
    released
}

/// Whether `node` and `record` were made together: the same author and date.
pub(crate) fn made_with(work: &Work, node: Id, record: Id) -> bool {
    attr(work, node, "author") == attr(work, record, "author") && attr(work, node, "date") == attr(work, record, "date")
}

/// Remove a property element a review emptied (a run's, a paragraph's, a mark's), and a
/// paragraph's properties it leaves empty in turn.
pub(crate) fn remove_if_empty(work: &mut Work, owner: Id) {
    if !work.children(owner).is_empty() || !work.is_any(owner, W, &["rPr", "pPr"]) {
        return;
    }
    let Some(parent) = work.parent(owner) else { return };
    if !work.is_any(parent, W, &["r", "p", "pPr"]) {
        return;
    }
    work.remove(owner);
    if work.is(parent, W, "pPr") {
        remove_if_empty(work, parent);
    }
}
