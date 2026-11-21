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
    let mut state = Walk::new(view, None);
    container(paragraph, &mut state);
    state.out
}

/// The runs that hold the paragraph's text in `view`, in document order, each with its place
/// below the paragraph, its first character (of any kind: a tab, an object) and how many
/// characters of `w:t` it holds (docx-agent's `walk(...).atoms` of kind `text`, counted per
/// run); a run holding none is left out.
#[must_use]
pub fn text_runs(paragraph: &Element, view: View) -> Vec<TextRun> {
    shown_runs(paragraph, view).into_iter().filter(|r| r.text > 0).collect()
}

/// The runs that show any character in `view`, of any kind, in document order (as
/// [`text_runs`], a run of only a tab or an object included): what docx-agent's
/// `run_text(paragraph, run)` is not empty for.
#[must_use]
pub fn shown_runs(paragraph: &Element, view: View) -> Vec<TextRun> {
    let mut state = Walk::new(view, None);
    state.runs = Some(Vec::new());
    container(paragraph, &mut state);
    state.runs.unwrap_or_default()
}

/// A run of a paragraph's text ([`text_runs`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRun {
    /// Its place below the paragraph.
    pub path: Box<[u32]>,
    /// Its first character in the view.
    pub first: char,
    /// Its first character of `w:t` in the view (`None`: it holds none).
    pub first_text: Option<char>,
    /// How many characters of `w:t` it holds in the view.
    pub text: usize,
    /// Whether any of its characters in the view, of any kind, is not white space (its text
    /// stripped is not empty).
    pub inked: bool,
}

/// How many characters of the paragraph's text in `view` stand before the element at
/// `marker` (its place below the paragraph): docx-agent's `_offset_of`, where a bookmark or
/// comment marker stands in its paragraph's text.
#[must_use]
pub fn offset_before(paragraph: &Element, marker: &[u32], view: View) -> usize {
    let mut state = Walk::new(view, Some(marker));
    container(paragraph, &mut state);
    state.before
}

struct Walk<'m> {
    view: View,
    /// Open complex fields, innermost last: `true` once in their result.
    fields: Vec<bool>,
    out: String,
    /// The place of the element being walked, below the paragraph.
    path: Vec<u32>,
    /// A marker's place, whose offset is counted in `before`.
    marker: Option<&'m [u32]>,
    /// Characters whose element stands before the marker in document order.
    before: usize,
    /// The runs written so far, when asked for ([`text_runs`]).
    runs: Option<Vec<TextRun>>,
}

impl<'m> Walk<'m> {
    fn new(view: View, marker: Option<&'m [u32]>) -> Self {
        Walk {
            view,
            fields: Vec::new(),
            out: String::new(),
            path: Vec::new(),
            marker,
            before: 0,
            runs: None,
        }
    }

    /// Text of the element at `self.path` written: counted when it stands before the marker.
    fn push(&mut self, text: &str) {
        self.out.push_str(text);
        if self.marker.is_some_and(|m| self.path.as_slice() < m) {
            self.before += text.chars().count();
        }
    }
}

fn step(k: usize) -> u32 {
    u32::try_from(k).unwrap_or(u32::MAX)
}

impl Walk<'_> {
    fn visible(&self) -> bool {
        self.fields.iter().all(|result| *result)
    }
}

fn is_w(element: &Element, names: &[&str]) -> bool {
    element.name().namespace() == Some(W) && names.contains(&element.name().local())
}

fn container(node: &Element, state: &mut Walk<'_>) {
    for (k, child) in node.children().iter().enumerate() {
        let Node::Element(child) = child else { continue };
        state.path.push(step(k));
        item(child, state);
        state.path.pop();
    }
}

fn item(child: &Element, state: &mut Walk<'_>) {
    {
        if is_w(child, &["r"]) {
            run(child, state);
        } else if is_w(
            child,
            &["hyperlink", "smartTag", "customXml", "dir", "bdo", "fldSimple"],
        ) {
            container(child, state);
        } else if is_w(child, &["sdt"]) {
            let content = child.children().iter().enumerate().find_map(|(k, n)| match n {
                Node::Element(e) if e.name().is(Some(W), "sdtContent") => Some((k, e)),
                _ => None,
            });
            if let Some((k, content)) = content {
                state.path.push(step(k));
                container(content, state);
                state.path.pop();
            }
        } else if let Some(shown) = state.view.shows(child) {
            if shown {
                container(child, state);
            }
        } else if (child.name().is(Some(M), "oMath") || child.name().is(Some(M), "oMathPara")) && state.visible() {
            state.push("\u{fffc}");
        }
    }
}

fn run(run: &Element, state: &mut Walk<'_>) {
    for (k, child) in run.children().iter().enumerate() {
        let Node::Element(child) = child else { continue };
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
            if let Some(runs) = state.runs.as_mut() {
                count(runs, &state.path, &c, name.is(Some(W), "t"));
            }
            state.path.push(step(k));
            match c {
                Char::One(c) => state.push(c.encode_utf8(&mut [0; 4])),
                Char::Text(t) => state.push(&t),
            }
            state.path.pop();
        }
    }
}

/// `c`, written in the run at `path`, counted in `runs` (`text`: a `w:t`'s characters).
fn count(runs: &mut Vec<TextRun>, path: &[u32], c: &Char, text: bool) {
    let (first, length) = match c {
        Char::One(c) => (Some(*c), 1),
        Char::Text(t) => (t.chars().next(), t.chars().count()),
    };
    let Some(first) = first else { return };
    if runs.last().is_none_or(|r| *r.path != *path) {
        runs.push(TextRun {
            path: path.into(),
            first,
            first_text: None,
            text: 0,
            inked: false,
        });
    }
    let inked = match c {
        Char::One(c) => !py_space(*c),
        Char::Text(t) => t.chars().any(|c| !py_space(c)),
    };
    if let Some(last) = runs.last_mut() {
        last.inked |= inked;
        if text {
            last.first_text.get_or_insert(first);
            last.text += length;
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

/// White space as Python's `str.isspace` says it (Rust's `White_Space` and the four
/// information separators).
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}
