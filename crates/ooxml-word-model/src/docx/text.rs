//! Owns: a paragraph's text in a view of its revisions, character for character as
//! docx-agent's `edit/text.py` walks it: `current` (insertions in, deletions out),
//! `original` (the reverse) and `markup` (both); a tab, a line or page break, a symbol and
//! an object are one character each, a field's instruction is not text.
//!
//! Must not: write text or read formatting.

use crate::ids::{MC, W};
use ooxml_xml::{Element, Node};

/// Math (`m:oMath`, an inline object).
const M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";

/// A line break (`w:br`, `w:cr`).
pub const LINE_BREAK: char = '\u{b}';
/// A page or column break.
pub const PAGE_BREAK: char = '\u{c}';
/// An object: a drawing, a note reference, an equation.
pub const OBJECT: char = '\u{fffc}';
/// `w:noBreakHyphen`.
pub const NON_BREAKING_HYPHEN: char = '\u{2011}';
/// `w:softHyphen`.
pub const SOFT_HYPHEN: char = '\u{ad}';

/// A view of a paragraph's revisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    /// Insertions in, deletions out.
    Current,
    /// Deletions in, insertions out.
    Original,
    /// Both.
    Markup,
}

impl View {
    /// Whether a revision container's content shows in this view.
    #[must_use]
    pub fn shows(self, container: &Element) -> Option<bool> {
        let local = container.name().local();
        if container.name().namespace() != Some(W) {
            return None;
        }
        match local {
            "ins" | "moveTo" => Some(self != View::Original),
            "del" | "moveFrom" => Some(self != View::Current),
            _ => None,
        }
    }
}

/// The paragraph's text in `view`.
#[must_use]
pub fn paragraph_text(paragraph: &Element, view: View) -> String {
    let mut state = Walk {
        view,
        fields: Vec::new(),
        out: String::new(),
    };
    container(paragraph, &mut state);
    state.out
}

struct Walk {
    view: View,
    /// Open complex fields, innermost last: `true` once in their result.
    fields: Vec<bool>,
    out: String,
}

impl Walk {
    fn visible(&self) -> bool {
        self.fields.iter().all(|result| *result)
    }
}

fn is_w(element: &Element, names: &[&str]) -> bool {
    element.name().namespace() == Some(W) && names.contains(&element.name().local())
}

fn container(node: &Element, state: &mut Walk) {
    for child in node.child_elements() {
        if is_w(child, &["r"]) {
            run(child, state);
        } else if is_w(
            child,
            &["hyperlink", "smartTag", "customXml", "dir", "bdo", "fldSimple"],
        ) {
            container(child, state);
        } else if is_w(child, &["sdt"]) {
            if let Some(content) = child.find(Some(W), "sdtContent") {
                container(content, state);
            }
        } else if let Some(shown) = state.view.shows(child) {
            if shown {
                container(child, state);
            }
        } else if (child.name().is(Some(M), "oMath") || child.name().is(Some(M), "oMathPara")) && state.visible() {
            state.out.push(OBJECT);
        }
    }
}

fn run(run: &Element, state: &mut Walk) {
    for child in run.child_elements() {
        let name = child.name();
        if name.is(Some(W), "rPr") {
            continue;
        }
        if name.is(Some(W), "fldChar") {
            match child.attribute(Some(W), "fldCharType") {
                Some("begin") => state.fields.push(false),
                Some("separate") => {
                    if let Some(last) = state.fields.last_mut() {
                        *last = true;
                    }
                }
                Some("end") => {
                    state.fields.pop();
                }
                _ => {}
            }
            continue;
        }
        if !state.visible() {
            continue;
        }
        if let Some(c) = character(child, state.view) {
            match c {
                Char::One(c) => state.out.push(c),
                Char::Text(t) => state.out.push_str(&t),
            }
        }
    }
}

enum Char {
    One(char),
    Text(String),
}

fn character(child: &Element, view: View) -> Option<Char> {
    let name = child.name();
    if name.namespace() == Some(MC) {
        return name.is(Some(MC), "AlternateContent").then_some(Char::One(OBJECT));
    }
    if name.namespace() != Some(W) {
        return None;
    }
    Some(match name.local() {
        "t" => Char::Text(leading_text(child)),
        "delText" if view != View::Current => Char::Text(leading_text(child)),
        "br" => Char::One(match child.attribute(Some(W), "type") {
            Some("page" | "column") => PAGE_BREAK,
            _ => LINE_BREAK,
        }),
        "tab" | "ptab" => Char::One('\t'),
        "cr" => Char::One(LINE_BREAK),
        "noBreakHyphen" => Char::One(NON_BREAKING_HYPHEN),
        "softHyphen" => Char::One(SOFT_HYPHEN),
        "sym" => Char::One(symbol(child)),
        "drawing" | "pict" | "object" | "footnoteReference" | "endnoteReference" | "footnoteRef" | "endnoteRef"
        | "contentPart" | "ruby" => Char::One(OBJECT),
        _ => return None,
    })
}

/// A `w:sym`'s character, or an object where its code is not a character.
fn symbol(node: &Element) -> char {
    node.attribute(Some(W), "char")
        .and_then(|c| u32::from_str_radix(c.trim(), 16).ok())
        .and_then(char::from_u32)
        .unwrap_or(OBJECT)
}

/// An element's text as lxml's `.text` reads it: the text before its first child element.
#[must_use]
pub fn leading_text(element: &Element) -> String {
    element.children().iter().map_while(Node::as_text).collect()
}
