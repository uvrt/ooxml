//! Owns: a comment on a whole content control, anchored as Word anchors one when the
//! control is selected (docx-agent's `_add_control_comment`, measured in Word 16.106):
//!
//! * in the line: the start right before the `w:sdt` (before its paragraph when the
//!   control opens it), the end right after it, then the reference run, at the size of the
//!   control's last character (or of the text before an empty one);
//! * a block, row or cell control: the start right before the `w:sdt`, displaced by it;
//!   the end and the reference at the end of the text of its last paragraph -- one
//!   character sooner when that paragraph is empty: at the end of the paragraph before it,
//!   or, that one empty too or none, the end before the paragraph and the reference
//!   opening it.
//!
//! Must not: write but into the edit's copy of the story ([`Space`]).

use super::anchors::reference_run;
use super::size::Sizes;
use crate::names::W;
use crate::review::space::Space;
use crate::review::xml::make;
use crate::text::atoms;
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::address::AddressError;
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::Docx;

/// The content control a comment covers: its part and place.
pub(super) struct Control {
    part: String,
    path: Vec<u32>,
    id: String,
}

/// The content control `identifier` names (`cc:...`, `cc@...`).
///
/// # Errors
/// `invalid_arguments` (docx-agent's `AddressError`), naming the document's controls, when
/// it has no such control.
pub(super) fn control(docx: &Docx, identifier: &str) -> Result<Control> {
    let all = controls(docx);
    if let Some(found) = all.iter().find(|c| c.id == identifier) {
        return Ok(Control {
            part: found.part.clone(),
            path: found.path.to_vec(),
            id: identifier.to_owned(),
        });
    }
    let known: Vec<String> = all.iter().map(|c| c.id.clone()).collect();
    let listed = if known.is_empty() {
        "none".to_owned()
    } else {
        let more = if known.len() > 20 {
            format!(" and {} more", known.len() - 20)
        } else {
            String::new()
        };
        format!(
            "{}{more}",
            known.iter().take(20).cloned().collect::<Vec<_>>().join(", ")
        )
    };
    Err(EditError::Address(AddressError::invalid(
        format!(
            "no content control {}: the document's are {listed}",
            ooxml_base::pyfmt::repr_str(identifier)
        ),
        known.into_iter().take(50).collect(),
    )))
}

/// Where the control `sdt` stands (`control_level`): `inline`, `block`, `row` or `cell`.
fn level(work: &Work, sdt: Id) -> &'static str {
    let mut parent = work.parent(sdt);
    while let Some(p) = parent.filter(|p| work.is_any(*p, W, &["customXml", "sdtContent", "smartTag"])) {
        parent = work.parent(p);
    }
    let Some(parent) = parent else { return "block" };
    let line = ["p", "hyperlink", "ins", "del", "moveTo", "moveFrom", "fldSimple", "sdt"];
    if work.is_any(parent, W, &line) {
        let holds_blocks = work
            .find(sdt, W, "sdtContent")
            .is_some_and(|c| work.find(c, W, "p").is_some() || work.find(c, W, "tbl").is_some());
        return if holds_blocks { "block" } else { "inline" };
    }
    if work.is(parent, W, "tbl") {
        return "row";
    }
    if work.is(parent, W, "tr") {
        return "cell";
    }
    "block"
}

/// Write comment `id`'s markers and reference run around the control `found`, in
/// `space`'s copy of its part.
///
/// # Errors
/// A control in the line outside a paragraph, or one holding no paragraph.
pub(super) fn mark_control(
    space: &mut Space,
    doc: &WordDoc,
    found: &Control,
    sizes: &Sizes<'_>,
    (id, reference): (i64, &str),
) -> Result<()> {
    let part = found.part.clone();
    space.work(doc, &part)?;
    let gone = || EditError::Path(part.clone());
    let sdt = space.locate(&part, &found.path).ok_or_else(gone)?;
    let work = space.get(&part).ok_or_else(gone)?;
    let inline = level(work, sdt) == "inline";
    let (size, paragraph, end_at, inside_text) = if inline {
        let mut at = work.parent(sdt);
        while let Some(p) = at.filter(|p| !work.is(*p, W, "p")) {
            at = work.parent(p);
        }
        let Some(paragraph) = at else {
            return Err(EditError::Refused(format!(
                "{} is not in a paragraph: no place for a comment",
                found.id
            )));
        };
        let items = atoms(work, paragraph);
        let held: Vec<usize> = (0..items.len())
            .filter(|k| items.get(*k).is_some_and(|a| within(work, a.node, sdt)))
            .collect();
        let offset = match held.last() {
            Some(last) => last + 1,
            None => {
                // an empty control: the size of the text before it
                let order = work.descendants(paragraph);
                let rank = |n: Id| order.iter().position(|o| *o == n);
                let at = rank(sdt);
                items.iter().filter(|a| rank(a.node) < at).count()
            }
        };
        (
            size_at(space, &part, paragraph, offset, sizes),
            paragraph,
            paragraph,
            true,
        )
    } else {
        let own = own_paragraphs(work, sdt);
        let Some(last) = own.last().copied() else {
            return Err(EditError::Refused(format!(
                "{} holds no paragraph to anchor a comment at",
                found.id
            )));
        };
        let (end_at, inside_text) = control_end(work, last);
        let offset = if inside_text { atoms(work, end_at).len() } else { 0 };
        (size_at(space, &part, end_at, offset, sizes), last, end_at, inside_text)
    };
    let work = space.get_mut(&part).ok_or_else(gone)?;
    let start = make(work, "commentRangeStart", &[("id", &id.to_string())]);
    let end = make(work, "commentRangeEnd", &[("id", &id.to_string())]);
    let run = reference_run(work, id, reference, size);
    if inline {
        place_start_in_line(work, sdt, paragraph, start);
        work.add_next(sdt, end);
        work.add_next(end, run);
    } else {
        let name = work.qname(W, "displacedByCustomXml", "w");
        work.set(start, name, "next");
        work.add_previous(sdt, start);
        if inside_text {
            let length = atoms(work, end_at).len();
            let (parent, at) = crate::inline::position(work, end_at, length)?;
            let (parent, at) = out_of_controls(work, parent, at);
            work.insert(parent, at, end);
            work.add_next(end, run);
        } else {
            place_before(work, end_at, end);
            let (parent, at) = crate::inline::position(work, end_at, 0)?;
            work.insert(parent, at, run);
        }
    }
    Ok(())
}

/// A comment's start before the control `sdt` in the line -- before its paragraph when
/// the control opens it.
fn place_start_in_line(work: &mut Work, sdt: Id, paragraph: Id, start: Id) {
    let opens = work.parent(sdt) == Some(paragraph)
        && work
            .children(paragraph)
            .iter()
            .take_while(|c| **c != sdt)
            .all(|c| !work.is_element(*c) || work.is(*c, W, "pPr"));
    if opens {
        place_before(work, paragraph, start);
    } else {
        work.add_previous(sdt, start);
    }
}

/// The size of the text before `offset` of `paragraph` in `space`'s copy of `part`.
fn size_at(space: &Space, part: &str, paragraph: Id, offset: usize, sizes: &Sizes<'_>) -> Option<i64> {
    let path = space.path_of(part, paragraph)?;
    let element = space.get(part)?.build_element(paragraph)?;
    sizes.at(part, &path, &element, offset)
}

/// Whether `node` is `ancestor` or below it.
fn within(work: &Work, node: Id, ancestor: Id) -> bool {
    let mut at = Some(node);
    while let Some(here) = at {
        if here == ancestor {
            return true;
        }
        at = work.parent(here);
    }
    false
}

/// The control's paragraphs in document order, without those of text boxes in it
/// (`_own_paragraphs`).
fn own_paragraphs(work: &Work, sdt: Id) -> Vec<Id> {
    work.iter(sdt, W, &["p"])
        .into_iter()
        .filter(|p| {
            let mut node = work.parent(*p);
            while let Some(n) = node.filter(|n| *n != sdt && !work.is(*n, W, "txbxContent")) {
                node = work.parent(n);
            }
            node == Some(sdt)
        })
        .collect()
}

/// Where a comment on a block control whose last paragraph is `last` ends
/// (`_control_end`): at the end of that paragraph's text (`true`), else -- an empty one --
/// at the end of the paragraph before it, or before `last` (`false`).
fn control_end(work: &Work, last: Id) -> (Id, bool) {
    if !atoms(work, last).is_empty() {
        return (last, true);
    }
    let mut before = work.previous(last);
    while let Some(b) = before.filter(|b| !work.is_element(*b)) {
        before = work.previous(b);
    }
    match before.filter(|b| work.is(*b, W, "p")) {
        Some(b) => (b, !atoms(work, b).is_empty()),
        None => (last, false),
    }
}

/// `marker` right before the block `node`; first in a control's content, marked displaced
/// by the control's start (`_place_before`).
fn place_before(work: &mut Work, node: Id, marker: Id) {
    work.add_previous(node, marker);
    let in_content = work.parent(node).is_some_and(|p| work.is(p, W, "sdtContent"));
    let first = work
        .parent(marker)
        .map(|p| work.children(p).to_vec())
        .unwrap_or_default()
        .into_iter()
        .take_while(|c| *c != marker)
        .all(|c| !work.is_element(c));
    if in_content && first {
        let name = work.qname(W, "displacedByCustomXml", "w");
        work.set(marker, name, "prev");
    }
}

/// A place at the end of an inline control's content moved to right after the control
/// (`_out_of_controls`): the end of a comment on a block control follows the controls in
/// its last line.
fn out_of_controls(work: &Work, mut parent: Id, mut at: usize) -> (Id, usize) {
    while work.is(parent, W, "sdtContent") && at == work.children(parent).len() {
        let Some(sdt) = work.parent(parent) else { break };
        if level(work, sdt) != "inline" {
            break;
        }
        let Some(above) = work.parent(sdt) else { break };
        at = work.index(sdt).map_or(0, |k| k + 1);
        parent = above;
    }
    (parent, at)
}
