//! Owns: a part's fields found where they are (docx-agent's `edit/fields.py` `scan`): each
//! complex field's begin, separator and end and each simple field by their paths in the
//! part's tree, its instruction and the fields nested in it, those inside a deletion left
//! out; a field's id (`fld:<story>/<n>`, its place among its story's fields), the paragraph
//! it begins in, its cached result as text (`_result_text`), and the document's fields in
//! story order (`_all_fields`).
//!
//! Must not: change a field or compute its value.

use crate::names::W;
use ooxml_word_model::docx::text::leading_text;
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// A field as the scan finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    /// Its place among its part's fields.
    pub(crate) number: usize,
    /// A `w:fldSimple`.
    pub(crate) simple: bool,
    /// Its instruction as written (`" PAGE "`).
    pub(crate) instruction: String,
    /// The path of its begin `w:fldChar`, or of its `w:fldSimple`.
    pub(crate) begin: Vec<u32>,
    /// The path of its separator.
    pub(crate) separate: Option<Vec<u32>>,
    /// The path of its end.
    pub(crate) end: Option<Vec<u32>>,
    /// The numbers of the fields nested in it.
    pub(crate) nested: Vec<usize>,
}

impl Found {
    /// Its kind: the instruction's first word, upper case (`PAGE`, `TOC`).
    pub(crate) fn keyword(&self) -> String {
        self.instruction
            .split_whitespace()
            .next()
            .map(str::to_uppercase)
            .unwrap_or_default()
    }
}

/// A field of the document: its part and story, and the field.
#[derive(Debug, Clone)]
pub(crate) struct Located {
    /// The story part.
    pub(crate) part: String,
    /// Its story's name.
    pub(crate) story: String,
    /// The field.
    pub(crate) found: Found,
}

impl Located {
    /// Its id: `fld:<story>/<n>`.
    pub(crate) fn id(&self) -> String {
        format!("fld:{}/{}", self.story, self.found.number)
    }
}

/// Every field of the part whose root is `root`, in document order, nested ones after
/// their outer one.
pub(crate) fn scan(root: &Element) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut path = Vec::new();
    walk(root, &mut path, false, &mut |node, path, deleted| {
        if deleted {
            return;
        }
        let name = node.name();
        if name.is(Some(W), "fldSimple") {
            let number = out.len();
            out.push(found(
                number,
                true,
                node.attribute(Some(W), "instr").unwrap_or(""),
                path,
            ));
        } else if name.is(Some(W), "instrText") {
            if let Some(field) = stack.last().and_then(|k| out.get_mut(*k)) {
                if field.separate.is_none() {
                    field.instruction.push_str(&leading_text(node));
                }
            }
        } else if name.is(Some(W), "fldChar") {
            match node.attribute(Some(W), "fldCharType") {
                Some("begin") => {
                    let number = out.len();
                    if let Some(outer) = stack.last().and_then(|k| out.get_mut(*k)) {
                        outer.nested.push(number);
                    }
                    stack.push(number);
                    out.push(found(number, false, "", path));
                }
                Some("separate") => {
                    if let Some(field) = stack.last().and_then(|k| out.get_mut(*k)) {
                        field.separate = Some(path.to_vec());
                    }
                }
                Some("end") => {
                    if let Some(field) = stack.pop().and_then(|k| out.get_mut(k)) {
                        field.end = Some(path.to_vec());
                    }
                }
                _ => {}
            }
        }
    });
    out
}

fn found(number: usize, simple: bool, instruction: &str, path: &[u32]) -> Found {
    Found {
        number,
        simple,
        instruction: instruction.to_owned(),
        begin: path.to_vec(),
        separate: None,
        end: None,
        nested: Vec::new(),
    }
}

/// Call `f` on every element below `element` in document order with its path and whether
/// it is inside a `w:del` or `w:moveFrom`.
fn walk(element: &Element, path: &mut Vec<u32>, deleted: bool, f: &mut impl FnMut(&Element, &[u32], bool)) {
    for (k, node) in element.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        f(child, path, deleted);
        let inside = deleted || child.name().is(Some(W), "del") || child.name().is(Some(W), "moveFrom");
        walk(child, path, inside, f);
        path.pop();
    }
}

/// Every field of the document, story by story in the reading's order.
pub(crate) fn all_fields(docx: &Docx) -> Vec<Located> {
    let mut out = Vec::new();
    for part in &docx.parts {
        for found in scan(part.root()) {
            out.push(Located {
                part: part.part.clone(),
                story: part.story.clone(),
                found,
            });
        }
    }
    out
}

/// The path of the paragraph holding the node at `path` (its nearest `w:p` above it).
pub(crate) fn paragraph_of(root: &Arc<Element>, path: &[u32]) -> Option<Vec<u32>> {
    let mut found = None;
    let mut node = root;
    for (k, step) in path.iter().enumerate() {
        node = node.children().get(*step as usize)?.as_element()?;
        if node.name().is(Some(W), "p") {
            found = Some(path.get(..=k)?.to_vec());
        }
    }
    found
}

/// The path of the run holding the node at `path` (its parent when that is a `w:r`, else
/// the node: `_run_of`).
pub(crate) fn run_of(root: &Arc<Element>, path: &[u32]) -> Vec<u32> {
    let (_, above) = path.split_last().unwrap_or((&0, &[]));
    match at_path(root, above) {
        Some(parent) if !above.is_empty() && parent.name().is(Some(W), "r") => above.to_vec(),
        _ => path.to_vec(),
    }
}

/// Whether `a` comes before `b` in document order (an ancestor before what it holds).
pub(crate) fn before(a: &[u32], b: &[u32]) -> bool {
    a < b
}

/// The cached result's text (`_result_text`): a simple field's text; a complex one's
/// between its separator and its end -- each paragraph a line, a run's tab a tab,
/// deleted text left out -- without the lines at its ends.
pub(crate) fn result_text(root: &Arc<Element>, found: &Found) -> String {
    if found.simple {
        return at_path(root, &found.begin).map_or_else(String::new, |e| texts(e));
    }
    let (Some(separate), Some(end)) = (&found.separate, &found.end) else {
        return String::new();
    };
    let (first, last) = (run_of(root, separate), run_of(root, end));
    let mut out = String::new();
    let mut path = Vec::new();
    between(
        root,
        &mut path,
        false,
        (&first, &last),
        &mut |node, _, deleted, parent_run| {
            let name = node.name();
            if name.is(Some(W), "t") && !deleted {
                out.push_str(&leading_text(node));
            } else if name.is(Some(W), "tab") && parent_run {
                out.push('\t');
            } else if name.is(Some(W), "p") {
                out.push('\n');
            }
        },
    );
    out.trim_matches('\n').to_owned()
}

/// Every `w:t`'s text below `element`, in order.
fn texts(element: &Element) -> String {
    let mut out = String::new();
    for node in element.children().iter() {
        let Node::Element(child) = node else { continue };
        if child.name().is(Some(W), "t") {
            out.push_str(&leading_text(child));
        } else {
            out.push_str(&texts(child));
        }
    }
    out
}

/// Call `f` on every element after `first` (not below it) and before `last` in document
/// order, with whether it is deleted and whether its parent is a run.
fn between(
    element: &Element,
    path: &mut Vec<u32>,
    deleted: bool,
    (first, last): (&[u32], &[u32]),
    f: &mut impl FnMut(&Element, &[u32], bool, bool),
) {
    let is_run = element.name().is(Some(W), "r");
    for (k, node) in element.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        let here: &[u32] = path;
        // past `last`: nothing further is wanted; `first` and below: skipped
        if !before(here, last) {
            path.pop();
            return;
        }
        let ends_before_first = before(here, first) && !first.starts_with(here);
        if !ends_before_first && !here.starts_with(first) {
            let inside = deleted || child.name().is(Some(W), "del") || child.name().is(Some(W), "moveFrom");
            if !before(here, first) {
                f(child, here, inside, is_run);
            }
            between(child, path, inside, (first, last), f);
        }
        path.pop();
    }
}
