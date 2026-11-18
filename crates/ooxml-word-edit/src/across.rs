//! Owns: a text range across paragraphs replaced, as docx-agent's `_replace_across` and
//! `_replace_across_tracked` do (measured on Word there): the range checked first
//! ([`check_across`]: one container, no table or control between, no section break, no
//! note reference or field cut in the paragraphs between); untracked, the text deleted and
//! the first and last paragraphs joined -- what is left of the first, and the new text,
//! moves to the start of the last, which keeps its element and id and takes the first
//! one's properties; tracked, the text deleted, the new text inserted where the first
//! paragraph's text ends, every mark between deleted and the last paragraph given the
//! first one's properties as a recorded change.
//!
//! The paragraphs live in a [`Work`] whose root stands for the stretch of their parent's
//! children they span; the caller splices the result back in.
//!
//! Must not: read outside the stretch, or allocate revision ids but through its stamp.

use crate::inline::{check_span, prune, relationship_ids};
use crate::marks::{delete_mark, take_properties};
use crate::names::W;
use crate::policy::Stamp;
use crate::sink::Keep;
use crate::text::{atoms, runs};
use crate::track::{delete_runs, delete_span};
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::EditError;

/// Range markers and their partners.
pub(crate) const PAIRS: &[(&str, &str)] = &[
    ("bookmarkStart", "bookmarkEnd"),
    ("commentRangeStart", "commentRangeEnd"),
    ("moveFromRangeStart", "moveFromRangeEnd"),
    ("moveToRangeStart", "moveToRangeEnd"),
    ("permStart", "permEnd"),
];

fn span_error(message: &str) -> EditError {
    EditError::Span(message.to_owned())
}

/// Whether a paragraph's field characters balance (docx-agent's `_fields_balanced`).
pub(crate) fn fields_balanced(work: &Work, paragraph: Id) -> bool {
    let mut depth = 0i64;
    for node in work.iter(paragraph, W, &["fldChar"]) {
        match work.get(node, Some(W), "fldCharType") {
            Some("begin") => depth += 1,
            Some("end") => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// Refuse a range across `paragraphs` (in order, children of the work's root) that the
/// join would break.
pub(crate) fn check_across(work: &Work, paragraphs: &[Id], start: usize, end: usize) -> Result<(), EditError> {
    let root = work.root();
    if paragraphs
        .iter()
        .any(|p| work.parent(*p) != Some(root) || !work.is(*p, W, "p"))
    {
        return Err(span_error("the range runs across a table, a cell or a content control"));
    }
    let blocks: Vec<Id> = work
        .children(root)
        .iter()
        .copied()
        .filter(|c| work.is_any(*c, W, &["p", "tbl", "sdt"]))
        .collect();
    if blocks != paragraphs {
        return Err(span_error("the range runs across a table or a content control"));
    }
    for p in paragraphs {
        if work
            .find(*p, W, "pPr")
            .and_then(|q| work.find(q, W, "sectPr"))
            .is_some()
        {
            return Err(span_error("the range runs across a section break"));
        }
    }
    let (Some(&first), Some(&last)) = (paragraphs.first(), paragraphs.last()) else {
        return Ok(());
    };
    let items = atoms(work, first);
    check_span(work, &items, start, items.len(), true)?;
    check_span(work, &atoms(work, last), 0, end, true)?;
    for p in paragraphs.get(1..paragraphs.len() - 1).unwrap_or_default() {
        for tag in ["footnoteReference", "endnoteReference", "commentReference"] {
            if !work.iter(*p, W, &[tag]).is_empty() {
                return Err(span_error(&format!("the range holds a paragraph with a {tag}")));
            }
        }
        if !fields_balanced(work, *p) {
            return Err(span_error("the range holds part of a field that spans paragraphs"));
        }
    }
    for p in paragraphs.get(..paragraphs.len() - 1).unwrap_or_default() {
        if !fields_balanced(work, *p) {
            return Err(span_error("the range holds part of a field that spans paragraphs"));
        }
    }
    Ok(())
}

/// The first paragraph after `element` in document order that is not inside it, else the
/// last one before it (docx-agent's `_neighbour_paragraph`, within the stretch).
pub(crate) fn neighbour(work: &Work, element: Id, forward: bool) -> Option<Id> {
    let all = work.iter(work.root(), W, &["p"]);
    let inside = work.iter(element, W, &["p"]);
    let first = all.iter().position(|p| inside.contains(p))?;
    let last = all.iter().rposition(|p| inside.contains(p))?;
    if forward {
        all.get(last + 1..)?.iter().copied().find(|p| !inside.contains(p))
    } else {
        all.get(..first)?.iter().rev().copied().find(|p| !inside.contains(p))
    }
}

/// Move the range markers in `element` whose partner is outside it to the nearest
/// paragraph that stays (docx-agent's `_rehome_markers`).
pub(crate) fn rehome_markers(work: &mut Work, element: Id) {
    let markers: Vec<(Id, String, String)> = work
        .descendants(element)
        .into_iter()
        .filter_map(|n| {
            let name = work.name(n)?;
            let local = name.local();
            let known = name.namespace() == Some(W) && PAIRS.iter().any(|(a, b)| *a == local || *b == local);
            known.then(|| (n, local.to_owned(), work.get(n, Some(W), "id").unwrap_or("").to_owned()))
        })
        .collect();
    // the last of a (tag, id) is the one docx-agent's dictionary keeps
    let mut kept: Vec<(Id, String, String)> = Vec::new();
    for marker in markers {
        match kept.iter_mut().find(|(_, t, i)| *t == marker.1 && *i == marker.2) {
            Some(slot) => *slot = marker,
            None => kept.push(marker),
        }
    }
    let partner = |local: &str| {
        PAIRS
            .iter()
            .find_map(|(a, b)| (*a == local).then_some(*b).or_else(|| (*b == local).then_some(*a)))
    };
    let lonely: Vec<Id> = kept
        .iter()
        .filter(|(_, t, i)| !kept.iter().any(|(_, u, j)| Some(u.as_str()) == partner(t) && j == i))
        .map(|(n, _, _)| *n)
        .collect();
    if lonely.is_empty() {
        return;
    }
    let following = neighbour(work, element, true);
    let previous = neighbour(work, element, false);
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

/// Untracked: the range `[start in the first, end in the last)` deleted, `text` in its
/// place, the first paragraph joined to the last. Returns the relationship ids released.
pub(crate) fn replace_across(
    work: &mut Work,
    paragraphs: &[Id],
    (start, end): (usize, usize),
    text: &str,
    keep: Keep,
) -> Result<Vec<String>, EditError> {
    let (Some(&first), Some(&last)) = (paragraphs.first(), paragraphs.last()) else {
        return Ok(Vec::new());
    };
    let mut released = crate::inline::replace_span(work, last, (0, end), "", Keep::First)?;
    for &middle in paragraphs.get(1..paragraphs.len() - 1).unwrap_or_default() {
        released.extend(relationship_ids(work, middle));
        rehome_markers(work, middle);
        work.remove(middle);
    }
    let length = atoms(work, first).len();
    released.extend(crate::inline::replace_span(work, first, (start, length), text, keep)?);
    let first_properties = work.find(first, W, "pPr");
    let mut at = 0;
    if let Some(properties) = work.find(last, W, "pPr") {
        work.remove(properties);
    }
    if let Some(properties) = first_properties {
        work.insert(last, 0, properties);
        at = 1;
    }
    for child in work.children(first).to_vec() {
        if Some(child) == first_properties {
            continue;
        }
        work.insert(last, at, child);
        at += 1;
    }
    work.remove(first);
    released.extend(prune(work, last));
    Ok(released)
}

/// Tracked: the text deleted, `text` inserted where the first paragraph's text ends, every
/// mark from the first to the one before the last deleted, and the last given the first
/// one's properties as a recorded change.
pub(crate) fn replace_across_tracked(
    work: &mut Work,
    paragraphs: &[Id],
    (start, end): (usize, usize),
    text: &str,
    keep: Keep,
    stamp: &mut Stamp,
    orders: &Orders,
) -> Result<Vec<String>, EditError> {
    let (Some(&first), Some(&last)) = (paragraphs.first(), paragraphs.last()) else {
        return Ok(Vec::new());
    };
    let mut released = delete_span(work, last, 0, end, stamp)?;
    let length = atoms(work, first).len();
    released.extend(crate::track::replace_span(
        work,
        first,
        (start, length),
        text,
        keep,
        stamp,
    )?);
    let properties = match work.find(first, W, "pPr") {
        Some(p) => work.deep_copy(p),
        None => work.new_element(work.element_name(first, W, "pPr", "w")),
    };
    for (k, &paragraph) in paragraphs
        .get(..paragraphs.len() - 1)
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        if k > 0 {
            let content = runs(work, paragraph);
            released.extend(delete_runs(work, &content, stamp));
        }
        delete_mark(work, paragraph, stamp, orders);
    }
    take_properties(work, last, properties, stamp, orders);
    Ok(released)
}
