//! Owns: what a review step leaves to clean across parts, as docx-agent's `_review` does
//! after its records (`revisions/review.py`): notes whose only reference it removed go
//! (`_drop_notes`), numbering instances it left unused go with the abstract definitions
//! only they used (`_reap_numbering`), and the numbering instances and authors in use
//! before it, which those steps compare with.
//!
//! Must not: apply records, or write but through the step's [`Space`].

use super::space::Space;
use super::xml::{attr, find_all};
use crate::doc::WordDoc;
use crate::names::W;
use crate::work::{Id, Work};
use crate::Result;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::parts::related;
use ooxml_xml::Element;
use std::collections::BTreeSet;

const REL_NUMBERING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
const REL_STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";

/// The parts numbering is referenced from: every story part, then the styles part.
fn numbered_parts(docx: &Docx) -> Vec<String> {
    let mut out: Vec<String> = docx.parts.iter().map(|p| p.part.clone()).collect();
    if let Ok(Some(styles)) = related(&docx.package, &docx.main, REL_STYLES) {
        out.push(styles);
    }
    out
}

/// Python's `int()` of a value, `None` when it is none.
fn number(value: Option<&str>) -> Option<i64> {
    value.and_then(|v| v.trim().parse().ok())
}

/// Every non-zero `numId` a `w:numPr` names below `element` (a green tree).
fn nums_in_tree(element: &Element, parent_is_num_pr: bool, out: &mut BTreeSet<i64>) {
    if parent_is_num_pr && element.name().is(Some(W), "numId") {
        if let Some(v) = number(element.attribute(Some(W), "val")).filter(|v| *v != 0) {
            out.insert(v);
        }
    }
    let here = element.name().is(Some(W), "numPr");
    for child in element.child_elements() {
        nums_in_tree(child, here, out);
    }
}

/// Every non-zero `numId` a `w:numPr` names in a copy.
fn nums_in_work(work: &Work, out: &mut BTreeSet<i64>) {
    for node in work.iter(work.root(), W, &["numId"]) {
        if work.parent(node).is_some_and(|p| work.is(p, W, "numPr")) {
            if let Some(v) = number(attr(work, node, "val")).filter(|v| *v != 0) {
                out.insert(v);
            }
        }
    }
}

/// Every `numId` a paragraph (any story) or a style names, as the step has left them
/// (`referenced_nums`).
pub(crate) fn referenced_nums(doc: &WordDoc, docx: &Docx, space: &Space) -> Result<BTreeSet<i64>> {
    let mut out = BTreeSet::new();
    for part in numbered_parts(docx) {
        match space.get(&part) {
            Some(work) => nums_in_work(work, &mut out),
            None => {
                if let Some(tree) = doc.tree(&part)? {
                    nums_in_tree(tree.root(), false, &mut out);
                }
            }
        }
    }
    Ok(out)
}

/// Remove the numbering instances a step left unused of those `before` named, and the
/// abstract definitions only they used (`_reap_numbering`).
pub(crate) fn reap_numbering(doc: &WordDoc, docx: &Docx, space: &mut Space, before: &BTreeSet<i64>) -> Result<()> {
    let after = referenced_nums(doc, docx, space)?;
    let gone: Vec<i64> = before.difference(&after).copied().collect();
    if gone.is_empty() {
        return Ok(());
    }
    let Ok(Some(part)) = related(&docx.package, &docx.main, REL_NUMBERING) else {
        return Ok(());
    };
    let Some(work) = space.work(doc, &part)? else {
        return Ok(());
    };
    let listed = |work: &Work, local: &str, attribute: &str| -> Vec<(i64, Id)> {
        let mut out: Vec<(i64, Id)> = Vec::new();
        for node in find_all(work, work.root(), local) {
            if let Some(n) = number(attr(work, node, attribute)) {
                out.retain(|(k, _)| *k != n);
                out.push((n, node));
            }
        }
        out
    };
    let abstract_of =
        |work: &Work, node: Id| number(work.find(node, W, "abstractNumId").and_then(|r| attr(work, r, "val")));
    let nums = listed(work, "num", "numId");
    let mut abstracts: Vec<i64> = Vec::new();
    for n in &gone {
        if let Some((_, node)) = nums.iter().find(|(k, _)| k == n) {
            if let Some(a) = abstract_of(work, *node) {
                if !abstracts.contains(&a) {
                    abstracts.push(a);
                }
            }
        }
    }
    for n in &gone {
        if let Some((_, node)) = nums.iter().find(|(k, _)| k == n) {
            work.remove(*node);
        }
    }
    let used: BTreeSet<i64> = listed(work, "num", "numId")
        .into_iter()
        .filter_map(|(_, node)| abstract_of(work, node))
        .collect();
    let definitions = listed(work, "abstractNum", "abstractNumId");
    for a in abstracts {
        if !used.contains(&a) {
            if let Some((_, node)) = definitions.iter().find(|(k, _)| *k == a) {
                work.remove(*node);
            }
        }
    }
    space.mark_dirty(&part);
    Ok(())
}

/// The note references of every story part as the step has left them.
fn referenced_notes(doc: &WordDoc, docx: &Docx, space: &Space) -> Result<BTreeSet<(String, String)>> {
    let mut out = BTreeSet::new();
    for story in &docx.parts {
        let mut add = |local: &str, id: Option<&str>| {
            let kind = local.trim_end_matches("Reference").to_owned();
            out.insert((kind, id.unwrap_or("").to_owned()));
        };
        match space.get(&story.part) {
            Some(work) => {
                for node in work.iter(work.root(), W, &["footnoteReference", "endnoteReference"]) {
                    let local = work.name(node).map(|n| n.local().to_owned()).unwrap_or_default();
                    add(&local, attr(work, node, "id"));
                }
            }
            None => {
                if let Some(tree) = doc.tree(&story.part)? {
                    let mut stack = vec![tree.root().clone()];
                    while let Some(e) = stack.pop() {
                        if e.name().namespace() == Some(W)
                            && matches!(e.name().local(), "footnoteReference" | "endnoteReference")
                        {
                            add(e.name().local(), e.attribute(Some(W), "id"));
                        }
                        stack.extend(e.child_elements().cloned());
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Remove the notes whose only reference the step removed; the relationship ids they held,
/// by part (`_drop_notes`).
pub(crate) fn drop_notes(
    doc: &WordDoc,
    docx: &Docx,
    space: &mut Space,
    notes: &BTreeSet<(String, String)>,
) -> Result<Vec<(String, Vec<String>)>> {
    let referenced = referenced_notes(doc, docx, space)?;
    let mut released = Vec::new();
    for story in &docx.parts {
        let local = story.root().name().local();
        if story.root().name().namespace() != Some(W) || !matches!(local, "footnotes" | "endnotes") {
            continue;
        }
        let Some(work) = space.work(doc, &story.part)? else {
            continue;
        };
        let mut removed_any = false;
        let mut rids = Vec::new();
        for note in work.children(work.root()).to_vec() {
            let Some(name) = work.name(note).filter(|n| n.namespace() == Some(W)) else {
                continue;
            };
            let key = (name.local().to_owned(), attr(work, note, "id").unwrap_or("").to_owned());
            let normal = matches!(attr(work, note, "type"), None | Some("normal"));
            if notes.contains(&key) && !referenced.contains(&key) && normal {
                let mut ids = crate::inline::relationship_ids(work, note);
                work.remove(note);
                removed_any = true;
                ids.sort();
                ids.dedup();
                rids.extend(ids);
            }
        }
        if removed_any {
            space.mark_dirty(&story.part);
            if !rids.is_empty() {
                released.push((story.part.clone(), rids));
            }
        }
    }
    Ok(released)
}

/// Every author of a revision or comment in the story parts (`_authors_in_use`).
pub(crate) fn authors_in_use(docx: &Docx) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for story in &docx.parts {
        let mut stack = vec![story.root().clone()];
        while let Some(e) = stack.pop() {
            let named = super::xml::RECORDS.contains(&e.name().local()) || e.name().local() == "comment";
            if named && e.name().namespace() == Some(W) {
                if let Some(author) = e.attribute(Some(W), "author") {
                    out.insert(author.to_owned());
                }
            }
            stack.extend(e.child_elements().cloned());
        }
    }
    out
}
