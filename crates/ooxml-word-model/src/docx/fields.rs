//! Owns: a part's fields as docx-agent scans them (`edit/fields.py`, `scan`): complex
//! fields (`w:fldChar` begin, separate, end, with the `w:instrText` before the separator)
//! and simple ones (`w:fldSimple`), nested ones after their outer, in document order, those
//! inside a deletion left out; and a field's keyword.
//!
//! Must not: compute a field's value or change a field.

use crate::docx::text::leading_text;
use crate::ids::W;
use ooxml_xml::{Element, Node};

/// A field found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Its instruction (`" PAGE "`).
    pub instruction: String,
    /// Whether it is a `w:fldSimple`.
    pub simple: bool,
}

impl Field {
    /// Its kind: the instruction's first word, upper case (`PAGE`, `REF`); `""` for none.
    #[must_use]
    pub fn keyword(&self) -> String {
        self.instruction
            .split_whitespace()
            .next()
            .map(str::to_uppercase)
            .unwrap_or_default()
    }
}

/// Every field of a part whose root is `root`, in document order.
#[must_use]
pub fn scan(root: &Element) -> Vec<Field> {
    let mut out: Vec<Field> = Vec::new();
    // The open complex fields: their place in `out` and whether they reached a separator.
    let mut stack: Vec<(usize, bool)> = Vec::new();
    visit(root, false, &mut |node, deleted| {
        if deleted {
            return;
        }
        let name = node.name();
        if name.is(Some(W), "fldSimple") {
            out.push(Field {
                instruction: node.attribute(Some(W), "instr").unwrap_or("").to_owned(),
                simple: true,
            });
        } else if name.is(Some(W), "instrText") {
            if let Some((at, false)) = stack.last() {
                if let Some(field) = out.get_mut(*at) {
                    field.instruction.push_str(&leading_text(node));
                }
            }
        } else if name.is(Some(W), "fldChar") {
            match node.attribute(Some(W), "fldCharType") {
                Some("begin") => {
                    stack.push((out.len(), false));
                    out.push(Field {
                        instruction: String::new(),
                        simple: false,
                    });
                }
                Some("separate") => {
                    if let Some(top) = stack.last_mut() {
                        top.1 = true;
                    }
                }
                Some("end") => {
                    stack.pop();
                }
                _ => {}
            }
        }
    });
    out
}

/// Call `f` on every element below `element` in document order, with whether it is inside
/// a `w:del` or `w:moveFrom`.
fn visit(element: &Element, deleted: bool, f: &mut impl FnMut(&Element, bool)) {
    for node in element.children().iter() {
        let Node::Element(child) = node else { continue };
        f(child, deleted);
        let inside = deleted || child.name().is(Some(W), "del") || child.name().is(Some(W), "moveFrom");
        visit(child, inside, f);
    }
}
