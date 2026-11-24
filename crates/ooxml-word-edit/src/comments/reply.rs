//! Owns: a reply's place in the stories, as docx-agent's `reply_to_comment` anchors one
//! (as Word anchors one): its range markers right after its thread's first comment's
//! (past any markers beside them), its reference run after that comment's reference runs,
//! at their size.
//!
//! Must not: write but into the edit's copies of the stories ([`Space`]).

use super::anchors::reference_run;
use crate::names::W;
use crate::review::space::Space;
use crate::review::xml::make;
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::docx::Docx;
use ooxml_xml::{Element, Node};

/// The markers a reply writes beside its parent's.
const MARKERS: [&str; 2] = ["commentRangeStart", "commentRangeEnd"];

/// Write reply `id`'s markers and reference runs beside those of the comment whose `w:id`
/// is `parent`, in every story holding them (the reference styled `reference`).
///
/// # Errors
/// A part that is not XML.
pub(super) fn mark_reply(
    space: &mut Space,
    doc: &WordDoc,
    docx: &Docx,
    parent: &str,
    (id, reference): (i64, &str),
) -> Result<()> {
    for story in &docx.parts {
        if !holds(story.root(), parent) {
            continue;
        }
        let part = story.part.clone();
        let work = space
            .work(doc, &part)?
            .ok_or_else(|| EditError::no_part(part.clone()))?;
        let root = work.root();
        for tag in MARKERS {
            for node in work.iter(root, W, &[tag]) {
                if work.get(node, Some(W), "id") != Some(parent) {
                    continue;
                }
                let marker = make(work, tag, &[("id", &id.to_string())]);
                let last = past(work, node, |w, n| w.is(n, W, tag));
                work.add_next(last, marker);
            }
        }
        for node in work.iter(root, W, &["commentReference"]) {
            if work.get(node, Some(W), "id") != Some(parent) {
                continue;
            }
            let Some(run) = work.parent(node) else { continue };
            let size = work
                .find(run, W, "rPr")
                .and_then(|p| work.find(p, W, "sz"))
                .and_then(|s| work.get(s, Some(W), "val"))
                .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()))
                .and_then(|v| v.parse::<i64>().ok());
            let new = reference_run(work, id, reference, size);
            let last = past(work, run, |w, n| w.find(n, W, "commentReference").is_some());
            work.add_next(last, new);
        }
    }
    Ok(())
}

/// The last of `node` and the siblings right after it that `like` holds for.
fn past(work: &Work, node: Id, like: impl Fn(&Work, Id) -> bool) -> Id {
    let mut last = node;
    while let Some(next) = work.next(last).filter(|n| like(work, *n)) {
        last = next;
    }
    last
}

/// Whether a comment marker or reference of `w:id` `id` is at or below `element`.
pub(super) fn holds(element: &Element, id: &str) -> bool {
    holds_any(
        element,
        id,
        &["commentRangeStart", "commentRangeEnd", "commentReference"],
    )
}

/// Whether some story holds a reference run of the comment whose `w:id` is `id`: a reply
/// then writes one (and the Comment Reference style is made first).
pub(super) fn has_reference(docx: &Docx, id: &str) -> bool {
    docx.parts
        .iter()
        .any(|s| holds_any(s.root(), id, &["commentReference"]))
}

/// Whether an element of Word's named one of `locals` with `w:id` `id` is at or below
/// `element`.
fn holds_any(element: &Element, id: &str, locals: &[&str]) -> bool {
    let named = element.name().namespace() == Some(W) && locals.contains(&element.name().local());
    if named && element.attribute(Some(W), "id") == Some(id) {
        return true;
    }
    element.children().iter().any(|c| match c {
        Node::Element(e) => holds_any(e, id, locals),
        _ => false,
    })
}
