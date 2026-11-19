//! Owns: the numbers content copied into the document takes where its own are in use, as
//! docx-agent's importer gives them (`edit/importing.py`, `Importer._drawing_ids` and
//! `_control_ids`): a drawing's `wp:docPr/@id` past every number the document has and had,
//! a content control's `w:sdtPr/w:id` a fresh value its old one and the document's count
//! of controls seed.
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
const CONTROL_RANGE: i64 = 2_000_000_000;

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

/// Give every content control in `element` whose id another control of the document
/// has (by absolute value) a new one (`_control_ids`): the first value of the sequence
/// seeded by its old id and the number of ids taken so far, within Word's range, stepped
/// past any taken. Reads every story part (the ids a control may not repeat).
///
/// # Errors
/// When the document cannot be read.
pub(crate) fn renumber_controls(sink: &mut EditSink<'_, '_>, work: &mut Work, element: Id) -> Result<()> {
    let nodes: Vec<Id> = work
        .iter(element, W, &["id"])
        .into_iter()
        .filter(|n| work.parent(*n).is_some_and(|p| work.is(p, W, "sdtPr")))
        .collect();
    if nodes.is_empty() {
        return Ok(());
    }
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
            work.set(node, Arc::new(QName::new(Some("w"), "val", Some(W))), &value.to_string());
        }
        taken.insert(value.abs());
    }
    Ok(())
}

/// A whole number as Python's `int` reads a string Word writes: surrounding white space
/// allowed, a sign, ASCII digits.
fn python_int(text: &str) -> Option<i64> {
    text.trim().parse().ok()
}
