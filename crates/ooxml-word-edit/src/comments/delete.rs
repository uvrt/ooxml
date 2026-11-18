//! Owns: deleting a comment with its replies as docx-agent does (`_delete_comment_now`,
//! `_delete_comments`): the comments, their range markers and reference runs in every
//! story, their entries in the modern parts (every modern part written again), and -- no
//! comment left -- the comment parts' relationships to release.
//!
//! Must not: tidy the people part (the command does, once, after its whole change).

use super::{part_of, REL_COMMENTS, REL_EXTENDED, REL_EXTENSIBLE, REL_IDS, W15, W16CEX, W16CID};
use crate::doc::WordDoc;
use crate::names::W;
use crate::review::space::Space;
use crate::work::{Id, Work};
use crate::Result;
use ooxml_word_model::docx::comments::Comment;
use ooxml_word_model::docx::Docx;
use std::collections::BTreeSet;

/// Delete `identifier` and its replies, as `comments` (the document's, as read before)
/// list them; nothing when it is already gone (in `gone`).
pub(crate) fn delete(
    doc: &WordDoc,
    docx: &Docx,
    space: &mut Space,
    comments: &[Comment],
    identifier: &str,
    gone: &mut BTreeSet<String>,
) -> Result<Vec<String>> {
    if gone.contains(identifier) || !comments.iter().any(|c| c.id == identifier) {
        return Ok(Vec::new());
    }
    let mut doomed = vec![identifier.to_owned()];
    let mut changed = true;
    while changed {
        changed = false;
        for c in comments {
            if c.parent.as_ref().is_some_and(|p| doomed.contains(p)) && !doomed.contains(&c.id) {
                doomed.push(c.id.clone());
                changed = true;
            }
        }
    }
    let records: Vec<&Comment> = doomed
        .iter()
        .filter_map(|d| comments.iter().find(|c| c.id == *d))
        .collect();
    let w_ids: BTreeSet<String> = records.iter().filter_map(|c| c.w_id.clone()).collect();
    let paras: BTreeSet<String> = records.iter().map(|c| c.last_para.clone()).collect();
    for story in &docx.parts {
        let Some(work) = space.work(doc, &story.part)? else {
            continue;
        };
        if remove_markers(work, &w_ids) {
            space.mark_dirty(&story.part);
        }
    }
    if let Some(part) = records.first().map(|c| c.part.clone()) {
        let elements: Vec<Option<Id>> = records.iter().map(|c| space.locate(&c.part, &c.path)).collect();
        if let Some(work) = space.get_mut(&part) {
            for element in elements.into_iter().flatten() {
                work.remove(element);
            }
        }
        space.mark_dirty(&part);
    }
    for (rel, namespace) in [(REL_EXTENDED, W15), (REL_IDS, W16CID)] {
        let Some(part) = part_of(&docx.package, &docx.main, rel) else {
            continue;
        };
        if let Some(work) = space.work(doc, &part)? {
            for item in work.children(work.root()).to_vec() {
                let para = work.get(item, Some(namespace), "paraId").unwrap_or("").to_uppercase();
                if paras.contains(&para) {
                    work.remove(item);
                }
            }
            space.mark_dirty(&part);
        }
    }
    let durables: BTreeSet<&str> = doomed.iter().filter_map(|d| d.strip_prefix("c:")).collect();
    if let Some(part) = part_of(&docx.package, &docx.main, REL_EXTENSIBLE) {
        if let Some(work) = space.work(doc, &part)? {
            for item in work.children(work.root()).to_vec() {
                if work
                    .get(item, Some(W16CEX), "durableId")
                    .is_some_and(|d| durables.contains(d))
                {
                    work.remove(item);
                }
            }
            space.mark_dirty(&part);
        }
    }
    gone.extend(doomed.iter().cloned());
    Ok(doomed)
}

/// Remove `w_ids`' range markers and references below the root of `work`, and the runs a
/// reference leaves with nothing but properties; whether any went.
fn remove_markers(work: &mut Work, w_ids: &BTreeSet<String>) -> bool {
    let mut touched = false;
    let found = work.iter(
        work.root(),
        W,
        &["commentRangeStart", "commentRangeEnd", "commentReference"],
    );
    for node in found {
        if !work.get(node, Some(W), "id").is_some_and(|id| w_ids.contains(id)) {
            continue;
        }
        let parent = work.parent(node);
        work.remove(node);
        touched = true;
        if let Some(run) = parent.filter(|p| work.is(*p, W, "r")) {
            let kept = work
                .children(run)
                .iter()
                .any(|c| work.is_element(*c) && !work.is(*c, W, "rPr"));
            if !kept {
                work.remove(run);
            }
        }
    }
    touched
}

/// Whether the comments part has no comment left (its parts then go, as Word writes it).
pub(crate) fn none_left(doc: &WordDoc, docx: &Docx, space: &Space) -> Result<bool> {
    let Some(part) = part_of(&docx.package, &docx.main, REL_COMMENTS) else {
        return Ok(false);
    };
    let left = match space.get(&part) {
        Some(work) => work.find(work.root(), W, "comment").is_some(),
        None => doc
            .tree(&part)?
            .is_some_and(|t| t.root().find(Some(W), "comment").is_some()),
    };
    Ok(!left)
}

/// The main part's relationship ids to the comment parts (`_drop_comment_part` of each).
pub(crate) fn comment_part_relationships(docx: &Docx) -> Vec<String> {
    let Ok(relationships) = docx.package.relationships(&docx.main) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for rel in [REL_COMMENTS, REL_EXTENDED, REL_IDS, REL_EXTENSIBLE] {
        let Some(part) = part_of(&docx.package, &docx.main, rel) else {
            continue;
        };
        for r in relationships.iter() {
            if r.target_part.as_deref() == Some(part.as_str()) && !out.contains(&r.id) {
                out.push(r.id.clone());
            }
        }
    }
    out
}
