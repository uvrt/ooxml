//! Owns: content controls (`w:sdt`) as docx-agent's `edit/controls.py` edits them --
//! [`InsertControl`] (a control in the line, as Word 365 writes one), [`FillControl`]
//! (filled as typing into it would: its text, a check box's state, a date's `w:fullDate`)
//! and [`RemoveControl`] (taken out, its content kept) -- and what they share: a control by
//! id, the paragraph a control in the line stands in, rewritten through the sink by the
//! policy (`rewrite`).
//!
//! Inserting and removing a control is no revision in Word (it has none for the wrapper):
//! tracked, the content's text is inserted or deleted, the wrapper is not, and the result
//! says so.
//!
//! Must not: write but through the sink; read more than the control's paragraph, or block
//! and its neighbours, except where docx-agent reads the whole document (a new control's
//! number is unlike every control's).

mod bound;
mod fill;
mod insert;
mod remove;

pub use fill::{FillControl, Filling};
pub use insert::{ControlKind, InsertControl};
pub use remove::RemoveControl;

use crate::doc::{Draw, DrawFrom, Key};
use crate::names::{W, W14, W16DU};
use crate::policy::RevisionPolicy;
use crate::set_text::{block_key, paragraph_has_annotations};
use crate::sink::{EditSink, TextWrite};
use crate::stamping::w14_prefix;
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::address::resolve::control;
use ooxml_word_model::docx::controls::{controls, Control};
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::QName;
use std::sync::Arc;

/// docx-agent's warning for a tracked insertion of a control.
const CONTENT_TRACKED: &str = "Word has no revision for a control itself: its content is tracked, the control is not";
/// docx-agent's warning for a tracked removal of a control.
const REMOVED_UNTRACKED: &str = "Word has no revision for a control itself: it is removed untracked";

/// What a content-control edit did, as docx-agent's `EditResult` says it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ControlEdited {
    /// The control it names (`cc:101`), if any.
    pub id: Option<String>,
    /// Ids it made.
    pub created: Vec<String>,
    /// Ids it removed.
    pub removed: Vec<String>,
    /// Ids renamed, old to new, in the order made.
    pub renamed: Vec<(String, String)>,
    /// What the edit says.
    pub warnings: Vec<String>,
}

/// The control `id` names now.
///
/// # Errors
/// `refused` for an id no control has (docx-agent's `_control`).
pub(crate) fn located(docx: &Docx, id: &str) -> Result<Control> {
    control(&controls(docx), id).map_err(EditError::Address)
}

/// The paragraph (with an id in its part) a control at `path` stands in: the nearest one
/// above it.
pub(crate) struct InParagraph {
    pub(crate) part: String,
    pub(crate) path: Box<[u32]>,
    pub(crate) id: String,
}

impl InParagraph {
    /// The paragraph nearest above the element at `path` of `part`.
    pub(crate) fn above(docx: &Docx, part: &str, path: &[u32]) -> Option<InParagraph> {
        let story = docx.part(part)?;
        (1..path.len()).rev().find_map(|n| {
            let prefix = path.get(..n)?;
            story.ids.paragraph_at(prefix).map(|p| InParagraph {
                part: part.to_owned(),
                path: p.path.clone(),
                id: p.id.clone(),
            })
        })
    }
}

/// How a paragraph's rewrite ends: its text id renewed (a paragraph whose text changed,
/// docx-agent's `_renew_text_id`), the part made to declare `w14` (`ensure_w14`).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Rewrite {
    pub(crate) renew: bool,
    pub(crate) ensure_w14: bool,
}

/// The paragraph `at` rewritten by `write` (given its work, the root the paragraph), by
/// the sink's policy: the text it writes marked for spacing, its revision ids drawn where
/// the change is applied, the people part told of the author.
///
/// # Errors
/// `write`'s; when the paragraph is not where it was found.
pub(crate) fn rewrite<R>(
    sink: &mut EditSink<'_, '_>,
    at: &InParagraph,
    how: Rewrite,
    write: impl FnOnce(&mut EditSink<'_, '_>, &mut Work) -> Result<(R, TextWrite)>,
) -> Result<R> {
    let part = at.part.as_str();
    let path_error = || EditError::Path(part.to_owned());
    let tree = sink
        .doc()
        .tree(part)?
        .ok_or_else(|| EditError::NoPart(part.to_owned()))?;
    let element = Arc::clone(at_path(tree.root(), &at.path).ok_or_else(path_error)?);
    let mut work = Work::of(&element, scope_above(tree.root(), &at.path));
    sink.load_written(part, Some(&at.path), &mut work);
    let (out, wrote) = write(sink, &mut work)?;
    let mut draws = Vec::new();
    let renew = how.renew && element.attribute(Some(W14), "paraId").is_some();
    if renew {
        let prefix = w14_prefix(tree.root(), &at.path);
        draws.push(Draw {
            from: DrawFrom::Seed(Arc::from(format!("{part}\0text"))),
            node: 0,
            at: Vec::new(),
            attribute: Arc::new(QName::new(Some(&prefix), "textId", Some(W14))),
        });
    }
    sink.keep_written(part, Some(&at.path), &work);
    let new = work.build().ok_or_else(path_error)?;
    if let Some(numbered) = wrote.annotations {
        draws.extend(crate::sink::annotation_draws(&new, numbered, 0));
    }
    let mut keys = vec![block_key(part, &at.id)];
    if paragraph_has_annotations(&element) || paragraph_has_annotations(&new) {
        keys.push(Key::Annotations);
    }
    sink.replace_drawing(part, &at.path, new, keys, draws)?;
    if wrote.dated {
        sink.ensure_w14(part, false, &[("w16du", W16DU)])?;
    }
    if how.ensure_w14 || renew {
        sink.ensure_w14(part, false, &[])?;
    }
    if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
        let gave = wrote.annotations.is_some_and(|n| n.to > n.from);
        sink.people(gave.then_some(tracking.author.as_str()), Some(tracking.author.as_str()))?;
    }
    sink.release(part, &wrote.released)?;
    Ok(out)
}

/// The element at `path` below `work`'s root.
pub(crate) fn at_place(work: &Work, path: &[u32]) -> Option<Id> {
    let mut node = work.root();
    for k in path {
        node = *work.children(node).get(*k as usize)?;
    }
    Some(node)
}

/// A `w:sdt`'s property element `local` of `namespace`, in `work`.
pub(crate) fn property_in(work: &Work, sdt: Id, namespace: &str, local: &str) -> Option<Id> {
    let properties = work.find(sdt, W, "sdtPr")?;
    work.find(properties, namespace, local)
}

/// A comment or bookmark mark Word wrote beside a control says so
/// (`w:displacedByCustomXml`, `next`/`prev`); with the control gone, a mark among `nodes`
/// no longer beside one says nothing (docx-agent's `_undisplace`).
pub(crate) fn undisplace(work: &mut Work, nodes: &[Id]) {
    for &node in nodes {
        let Some(side) = work.get(node, Some(W), "displacedByCustomXml").map(str::to_owned) else {
            continue;
        };
        let Some(parent) = work.parent(node) else { continue };
        let neighbour = if side == "next" {
            work.next(node)
        } else {
            work.previous(node)
        };
        if neighbour.is_none() && side == "prev" && work.is(parent, W, "sdtContent") {
            continue; // first in another control's content: still beside its start
        }
        if !neighbour.is_some_and(|n| work.is_any(n, W, &["sdt", "customXml"])) {
            work.remove_attr(node, Some(W), "displacedByCustomXml");
        }
    }
}
