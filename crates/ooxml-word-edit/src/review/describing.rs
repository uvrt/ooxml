//! Owns: the comments docx-agent wrote (before its round 57) to describe a structural
//! table change -- their text opens with [`DESCRIBES`] -- that a review step leaves with
//! nothing to describe (docx-agent's `_describe_resolved`): one whose range starts in no
//! table, or in a table with no cell record left (`w:cellIns`, `w:cellDel`,
//! `w:cellMerge`, `w:tcPrChange`).
//!
//! Must not: delete a comment (the step does, as it deletes the others), or read the
//! table but as the step left it.

use super::space::Space;
use crate::doc::WordDoc;
use crate::names::W;
use crate::work::Work;
use crate::Result;
use ooxml_word_model::docx::comments::Comment;
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::{Element, Node};
use std::collections::BTreeSet;

/// What a describing comment's text opens with (docx-agent's `DESCRIBES`).
pub(crate) const DESCRIBES: &str = "Tracked structural change:";

/// A table's records that keep a description (`_CELL` and `w:tcPrChange`).
const CELL_RECORDS: [&str; 4] = ["cellIns", "cellDel", "cellMerge", "tcPrChange"];

/// The ids of the describing comments of `listed` (not `gone`) whose table has no cell
/// record left in `space` (the step's copies; a part it has none of as `doc` has it).
///
/// # Errors
/// When a part cannot be read.
pub(crate) fn resolved(
    doc: &WordDoc,
    docx: &Docx,
    space: &mut Space,
    listed: &[Comment],
    gone: &BTreeSet<String>,
) -> Result<Vec<String>> {
    let describing: Vec<&Comment> = listed
        .iter()
        .filter(|c| !gone.contains(&c.id) && says(space, docx, c).starts_with(DESCRIBES))
        .collect();
    let mut out = Vec::new();
    for comment in describing {
        let Some(w_id) = comment.w_id.as_deref() else {
            out.push(comment.id.clone()); // no range names it: no table
            continue;
        };
        // the last range start found names the table, as docx-agent's walk leaves it
        let mut table: Option<bool> = None;
        for story in &docx.parts {
            let Some(work) = space.work(doc, &story.part)? else {
                continue;
            };
            for start in work.iter(work.root(), W, &["commentRangeStart"]) {
                if work.get(start, Some(W), "id") == Some(w_id) {
                    table = Some(table_has_cell_records(work, start));
                }
            }
        }
        if table != Some(true) {
            out.push(comment.id.clone());
        }
    }
    Ok(out)
}

/// Whether the nearest table above `node` holds a cell record (`false`: no table).
fn table_has_cell_records(work: &Work, node: crate::work::Id) -> bool {
    let mut at = work.parent(node);
    while let Some(id) = at {
        if work.is(id, W, "tbl") {
            return !work.iter(id, W, &CELL_RECORDS).is_empty();
        }
        at = work.parent(id);
    }
    false
}

/// A comment's `w:t` text, joined (as the step left it, where it has a copy of the part).
fn says(space: &Space, docx: &Docx, comment: &Comment) -> String {
    if let (Some(work), Some(id)) = (space.get(&comment.part), space.locate(&comment.part, &comment.path)) {
        return work
            .iter(id, W, &["t"])
            .into_iter()
            .map(|t| work.text(t).to_owned())
            .collect();
    }
    docx.part(&comment.part)
        .and_then(|p| at_path(p.root(), &comment.path))
        .map(|e| text_of(e))
        .unwrap_or_default()
}

fn text_of(element: &Element) -> String {
    let mut out = String::new();
    for node in element.children().iter() {
        if let Node::Element(child) = node {
            if child.name().namespace() == Some(W) && child.name().local() == "t" {
                for t in child.children().iter() {
                    if let Node::Text(text) = t {
                        out.push_str(text);
                    }
                }
            } else {
                out.push_str(&text_of(child));
            }
        }
    }
    out
}
