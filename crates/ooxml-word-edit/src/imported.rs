//! Owns: the numbers content copied into the document takes where its own are in use, as
//! docx-agent's importer gives them (`edit/importing.py`, `Importer._drawing_ids` and
//! `_control_ids`): a drawing's `wp:docPr/@id` past every number the document has and had,
//! a content control's `w:sdtPr/w:id` a fresh value its old one and the count of ids
//! taken seed ([`Controls`], one per edit); a copy from another document keeping its
//! drawing numbers where the document never had them.
//!
//! Must not: place the content, stamp its paragraph or drawing ids (the caller does), or
//! read more of the document than the numbers it compares with.

use crate::names::{W, WP};
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::Result;
use ooxml_word_model::docx::child_val;
use ooxml_word_model::ids::sequence_value;
use ooxml_xml::QName;
use std::collections::HashSet;
use std::sync::Arc;

/// Content-control ids are kept within this (docx-agent's `% 2_000_000_000 + 1`).
pub(crate) const CONTROL_RANGE: i64 = 2_000_000_000;

/// Give every drawing in `element`, a copy of the document's own content (each number it
/// carries is in use), a new number: past every one the document has and had, in document
/// order (`_drawing_ids` for a number in use).
///
/// # Errors
/// When the draw cannot be recorded.
pub(crate) fn renumber_drawings(sink: &mut EditSink<'_, '_>, work: &mut Work, element: Id) -> Result<()> {
    let nodes = work.iter(element, WP, &["docPr"]);
    let numbers = sink.draw_drawings(nodes.len())?;
    for (node, number) in nodes.into_iter().zip(numbers) {
        work.set(node, Arc::new(QName::new(None, "id", None)), &number.to_string());
    }
    Ok(())
}

/// Number the drawings of `elements`, a copy from another document (or this one as it
/// was), as docx-agent's importer does (`_drawing_ids`): in document order, each keeps its
/// `wp:docPr/@id` where the document never had that number and the copy has not given it,
/// the others the number after every one given.
///
/// # Errors
/// When the numbers cannot be recorded.
pub(crate) fn keep_or_renumber_drawings(sink: &mut EditSink<'_, '_>, work: &mut Work, elements: &[Id]) -> Result<()> {
    let nodes: Vec<Id> = elements.iter().flat_map(|e| work.iter(*e, WP, &["docPr"])).collect();
    let values: Vec<Option<i64>> = nodes
        .iter()
        .map(|n| work.get(*n, None, "id").and_then(python_int))
        .collect();
    let numbers = sink.keep_or_draw_drawings(&values)?;
    for ((node, value), number) in nodes.into_iter().zip(values).zip(numbers) {
        if value != Some(number) {
            work.set(node, Arc::new(QName::new(None, "id", None)), &number.to_string());
        }
    }
    Ok(())
}

/// Give every content control in `element` whose id another control of the document
/// has (by absolute value) a new one (`_control_ids` of one piece of content).
///
/// # Errors
/// When the document cannot be read.
pub(crate) fn renumber_controls(sink: &mut EditSink<'_, '_>, work: &mut Work, element: Id) -> Result<()> {
    Controls::default().renumber(sink, work, &[element]).map(|_| ())
}

/// The content-control ids one edit's copied content takes, as docx-agent's importer
/// keeps them (`_control_ids`): the ids every story's controls have, read once the edit
/// meets its first control, and each id the copies took since.
#[derive(Default)]
pub(crate) struct Controls {
    /// The absolute values taken.
    taken: Option<HashSet<i64>>,
}

impl Controls {
    /// Give every content control in `elements` whose id is taken (by absolute value) a
    /// new one: the first value of the sequence seeded by its old id and the number of ids
    /// taken so far, within Word's range, stepped past any taken. Reads every story part
    /// (the ids a control may not repeat). Each id changed, old to new, in order.
    ///
    /// # Errors
    /// When the document cannot be read.
    pub(crate) fn renumber(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        work: &mut Work,
        elements: &[Id],
    ) -> Result<Vec<(i64, i64)>> {
        let nodes: Vec<Id> = elements
            .iter()
            .flat_map(|e| work.iter(*e, W, &["id"]))
            .filter(|n| work.parent(*n).is_some_and(|p| work.is(p, W, "sdtPr")))
            .collect();
        let mut changed = Vec::new();
        if nodes.is_empty() {
            return Ok(changed);
        }
        if self.taken.is_none() {
            self.taken = Some(taken_ids(sink)?);
        }
        let taken = self.taken.get_or_insert_with(HashSet::new);
        for node in nodes {
            let Some(mut value) = work.get(node, Some(W), "val").and_then(python_int) else {
                continue;
            };
            if taken.contains(&value.abs()) {
                let old = value;
                let first = (0..)
                    .map(|k| sequence_value(&format!("control\0{old}\0{}", taken.len()), k))
                    .find(|v| *v != 0)
                    .unwrap_or(1);
                value = i64::from(first) % CONTROL_RANGE + 1;
                while taken.contains(&value) {
                    value = (value * 7 + 13) % CONTROL_RANGE + 1;
                }
                work.set(
                    node,
                    Arc::new(QName::new(Some("w"), "val", Some(W))),
                    &value.to_string(),
                );
                changed.push((old, value));
            }
            taken.insert(value.abs());
        }
        Ok(changed)
    }
}

/// The absolute ids of every story's content controls (the whole document read).
fn taken_ids(sink: &mut EditSink<'_, '_>) -> Result<HashSet<i64>> {
    sink.read_everything();
    let docx = sink.docx()?;
    let mut taken: HashSet<i64> = HashSet::new();
    for part in &docx.parts {
        for found in part.facts().controls() {
            let value = found
                .element
                .find(Some(W), "sdtPr")
                .and_then(|p| child_val(p, "id"))
                .and_then(python_int);
            if let Some(value) = value {
                taken.insert(value.abs());
            }
        }
    }
    Ok(taken)
}

/// A whole number as Python's `int` reads a string Word writes: surrounding white space
/// allowed, a sign, ASCII digits.
pub(crate) fn python_int(text: &str) -> Option<i64> {
    text.trim().parse().ok()
}
