//! Owns: a new content control's number (docx-agent's `_next_control_id`, unlike every
//! control's in the document) and what choosing it depends on, read by condition
//! ([`NumbersControl`]): another edit giving a control that number meanwhile.
//!
//! Must not: write anything.

use crate::doc::Key;
use crate::imported::{python_int, CONTROL_RANGE};
use crate::names::W;
use crate::WordDoc;
use ooxml_store::ReadPredicate;
use ooxml_word_model::docx::{at_path, child_val, Docx, StoryPart};
use ooxml_word_model::ids::{sequence_value, Entry};
use ooxml_xml::Element;
use std::collections::HashSet;

/// The number a new control takes (`_next_control_id`): the first of the
/// `content-control` sequence within Word's range, stepped past any control's (or its
/// negative).
pub(super) fn next_number(docx: &Docx) -> i64 {
    let mut used: HashSet<i64> = HashSet::new();
    for part in &docx.parts {
        for found in part.facts().controls() {
            used.extend(number_of(&found.element));
        }
    }
    let first = (0..)
        .map(|k| sequence_value("content-control", k))
        .find(|v| *v != 0)
        .unwrap_or(1);
    let mut number = i64::from(first) % CONTROL_RANGE + 1;
    while used.contains(&number) || used.contains(&-number) {
        number = (number * 7 + 13) % CONTROL_RANGE + 1;
    }
    number
}

/// A `w:sdt`'s number (`w:sdtPr/w:id`).
fn number_of(sdt: &Element) -> Option<i64> {
    sdt.find(Some(W), "sdtPr")
        .and_then(|p| child_val(p, "id"))
        .and_then(python_int)
}

/// A new control's number read by condition (design section 5): it depends on no control
/// having that number (or its negative). A block written since the edit's snapshot
/// conflicts when, on the head, a control in it or around it has the number; a story part
/// written whole when one of its controls has it; nothing else does.
///
/// A control removed meanwhile is no conflict: the number stays unlike every control's,
/// though the serial order might have chosen the removed one's.
#[derive(Debug)]
pub(super) struct NumbersControl {
    /// The number chosen.
    pub(super) number: i64,
}

impl NumbersControl {
    fn numbers(&self, sdt: &Element) -> bool {
        number_of(sdt).is_some_and(|n| n == self.number || n == -self.number)
    }

    /// Whether `element` is or holds a control of the number.
    fn holds(&self, element: &Element) -> bool {
        if element.name().is(Some(W), "sdt") && self.numbers(element) {
            return true;
        }
        element.child_elements().any(|child| self.holds(child))
    }

    /// Whether block `id` of `story`, or a control around it, has the number; a block the
    /// index does not know (a row, a block gone) may.
    fn near(&self, story: &StoryPart, id: &str) -> bool {
        let ids = &story.ids;
        let path = match ids.get(id) {
            Some(Entry::Paragraph(k)) => ids.paragraphs().get(k).map(|p| p.path),
            Some(Entry::Table(k)) => ids.tables().get(k).map(|t| t.path),
            None => None,
        };
        let Some(path) = path else { return true };
        let Some(block) = at_path(story.root(), &path) else {
            return true;
        };
        if self.holds(block) {
            return true;
        }
        // a block-level control the block stands in
        (1..path.len()).any(|n| {
            path.get(..n)
                .and_then(|above| at_path(story.root(), above))
                .is_some_and(|e| e.name().is(Some(W), "sdt") && self.numbers(e))
        })
    }
}

impl ReadPredicate<WordDoc> for NumbersControl {
    fn depends(&self, head: &WordDoc, key: &Key) -> bool {
        let Ok(docx) = head.docx() else { return true };
        match key {
            Key::Block { part, id } => docx.part(part).is_some_and(|story| self.near(story, id)),
            Key::Part(part) => docx
                .part(part)
                .is_some_and(|story| story.facts().controls().iter().any(|c| self.numbers(&c.element))),
            Key::Root(_) | Key::Seq(_) | Key::Annotations | Key::People | Key::StyleUse(_) => false,
        }
    }
}
