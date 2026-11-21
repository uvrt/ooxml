//! Owns: what the layout facts read of the document's tree: the body's top-level blocks
//! with their layout paths, places and ids ([`top_level`], docx-agent's `_top_level`), the
//! top-level block a line's path is in, the table and row a line of a body table's row
//! names (`_ROW`), a layout path's element in its part, a paragraph's style, and the
//! paragraph tests the facts share (`_page_break_at`, `_CONTENT`).
//!
//! Must not: read the layout.

use ooxml_word_model::docx::{Docx, StoryPart};
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// The WordprocessingML namespace.
pub(crate) const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
/// The markup compatibility namespace.
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// What a top-level block is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// A paragraph.
    Paragraph,
    /// A table.
    Table,
}

/// One of the body's top-level blocks.
#[derive(Debug, Clone)]
pub struct Block<'a> {
    /// A paragraph or a table.
    pub kind: BlockKind,
    /// Its element.
    pub element: &'a Arc<Element>,
    /// Its place in the main part (child indexes from the root).
    pub place: Vec<u32>,
    /// Its layout path.
    pub path: String,
    /// Its id.
    pub id: String,
}

/// Whether `e` is the `w:` element `local`.
pub(crate) fn is_w(e: &Element, local: &str) -> bool {
    e.name().is(Some(W), local)
}

/// The body's paragraphs and tables in order, block-level content controls opened: those
/// the part's ids name.
pub(crate) fn top_level(docx: &Docx) -> Vec<Block<'_>> {
    let mut out = Vec::new();
    let Some(story) = docx.body() else { return out };
    let root = story.root();
    let Some((k, body)) = children(root).find(|(_, _, e)| is_w(e, "body")).map(|(k, _, e)| (k, e)) else {
        return out;
    };
    visit(story, body, &mut vec![k], "w:body", &mut out);
    out
}

/// The element children of `element` the layout counts, with their child index and their
/// count among the counted siblings of their local name (from 1).
fn children(element: &Arc<Element>) -> impl Iterator<Item = (u32, u32, &Arc<Element>)> + '_ {
    let mut seen: HashMap<&str, u32> = HashMap::new();
    element.children().iter().enumerate().filter_map(move |(k, node)| {
        let Node::Element(child) = node else { return None };
        if child.name().is(Some(MC), "Fallback") {
            return None;
        }
        let count = seen.entry(child.name().local()).or_insert(0);
        *count += 1;
        Some((u32::try_from(k).unwrap_or(u32::MAX), *count, child))
    })
}

fn visit<'a>(
    story: &'a StoryPart,
    container: &'a Arc<Element>,
    place: &mut Vec<u32>,
    path: &str,
    out: &mut Vec<Block<'a>>,
) {
    for (k, count, child) in children(container) {
        place.push(k);
        let here = format!("{path}/w:{}[{count}]", child.name().local());
        if is_w(child, "sdt") {
            let content = children(child).find(|(_, _, e)| is_w(e, "sdtContent"));
            if let Some((c, n, content)) = content {
                place.push(c);
                visit(story, content, place, &format!("{here}/w:sdtContent[{n}]"), out);
                place.pop();
            }
        } else if is_w(child, "p") || is_w(child, "tbl") {
            let id = story.ids.at(place).and_then(|entry| story.ids.id_of(entry));
            if let Some(id) = id {
                let kind = if is_w(child, "p") {
                    BlockKind::Paragraph
                } else {
                    BlockKind::Table
                };
                out.push(Block {
                    kind,
                    element: child,
                    place: place.clone(),
                    path: here,
                    id,
                });
            }
        }
        place.pop();
    }
}

/// The top-level block a body line's path is in: its steps up to the first paragraph or
/// table below the body and its block-level content controls.
pub(crate) fn top_block_path(path: &str) -> Option<&str> {
    let mut end = 0;
    for step in path.split('/') {
        end += step.len();
        let local = step.split('[').next().unwrap_or(step);
        if local == "w:p" || local == "w:tbl" {
            return path.get(..end);
        }
        end += 1;
    }
    None
}

/// Every table a body line's path is in, outermost first: its path's prefixes ending at a
/// `w:tbl` step.
pub(crate) fn table_paths(path: &str) -> impl Iterator<Item = &str> + '_ {
    let mut end = 0;
    path.split('/').filter_map(move |step| {
        let at = end + step.len();
        end = at + 1;
        step.starts_with("w:tbl[").then(|| path.get(..at)).flatten()
    })
}

/// A line of a body table's row, as docx-agent's `_ROW` reads it: the outermost table's
/// path (`w:body/w:tbl[n]`, its content controls written `w:sdt[n]/w:sdtContent`) and the
/// row, from 0.
pub(crate) fn row_of(path: &str) -> Option<(&str, usize)> {
    let rest = path.strip_prefix("w:body/")?;
    let mut at = "w:body/".len();
    let mut steps = rest.split('/');
    loop {
        let step = steps.next()?;
        at += step.len();
        if indexed(step, "w:sdt").is_some() {
            if steps.next()? != "w:sdtContent" {
                return None;
            }
            at += 1 + "w:sdtContent".len() + 1;
            continue;
        }
        indexed(step, "w:tbl")?;
        let table = path.get(..at)?;
        let row = indexed(steps.next()?, "w:tr")?;
        return Some((table, row.checked_sub(1)?));
    }
}

/// The count `n` of a step `name[n]` (digits only).
fn indexed(step: &str, name: &str) -> Option<usize> {
    let digits = step.strip_prefix(name)?.strip_prefix('[')?.strip_suffix(']')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The element a layout path names in `story` and its place (child indexes from the
/// root): the path's first step the root itself where it names the root's kind.
pub(crate) fn resolve<'a>(story: &'a StoryPart, path: &str) -> Option<(&'a Arc<Element>, Vec<u32>)> {
    let mut node = story.root();
    let mut place = Vec::new();
    let mut steps = path.split('/').peekable();
    let first = steps.peek()?.split('[').next().unwrap_or_default();
    if first.strip_prefix("w:") == Some(node.name().local()) {
        steps.next();
    }
    for step in steps {
        let (name, count) = match step.split_once('[') {
            Some((name, count)) => (name, count.strip_suffix(']')?.parse::<u32>().ok()?),
            None => (step, 1),
        };
        let local = name.strip_prefix("w:")?;
        let (k, _, child) = children(node).find(|(_, n, e)| e.name().local() == local && *n == count)?;
        place.push(k);
        node = child;
    }
    Some((node, place))
}

/// A paragraph's `w:pStyle`: `None` where it has none, `Some(None)` where it names no
/// value.
pub(crate) fn paragraph_style(element: &Element) -> Option<Option<String>> {
    let node = element.find(Some(W), "pPr")?.find(Some(W), "pStyle")?;
    Some(node.attribute(Some(W), "val").map(str::to_owned))
}

/// Every element below `element` and itself, in document order.
pub(crate) fn descendants(element: &Arc<Element>) -> Vec<&Arc<Element>> {
    let mut out = Vec::new();
    let mut stack = vec![element];
    while let Some(node) = stack.pop() {
        out.push(node);
        let below: Vec<&Arc<Element>> = node.child_elements().collect();
        stack.extend(below.into_iter().rev());
    }
    out
}

/// Whether a paragraph's first (`start`) or last content -- text that is not blank, a
/// break, a tab or a drawing -- is a page break (`_page_break_at`).
pub(crate) fn page_break_at(paragraph: &Arc<Element>, start: bool) -> bool {
    let all = descendants(paragraph);
    let mut content = all.iter().filter(|e| {
        (is_w(e, "t") && !py_strip(&e.text_content()).is_empty())
            || is_w(e, "br")
            || is_w(e, "tab")
            || is_w(e, "drawing")
    });
    let found = if start { content.next() } else { content.last() };
    found.is_some_and(|e| is_w(e, "br") && e.attribute(Some(W), "type") == Some("page"))
}

/// What makes an empty-looking paragraph not empty: something drawn or a break
/// (`_CONTENT`).
const CONTENT: [&str; 14] = [
    "drawing",
    "pict",
    "object",
    "br",
    "cr",
    "sym",
    "tab",
    "ptab",
    "fldChar",
    "fldSimple",
    "footnoteReference",
    "endnoteReference",
    "sectPr",
    "framePr",
];

/// Whether `element` or anything below it is [`CONTENT`].
pub(crate) fn has_content(element: &Arc<Element>) -> bool {
    descendants(element)
        .iter()
        .any(|e| e.name().namespace() == Some(W) && CONTENT.contains(&e.name().local()))
}

/// The text of every `w:t` below `element`, stripped as Python strips it.
pub(crate) fn stripped_text(element: &Arc<Element>) -> String {
    let joined: String = descendants(element)
        .iter()
        .filter(|e| is_w(e, "t"))
        .map(|e| e.text_content())
        .collect();
    py_strip(&joined).to_owned()
}

/// White space as Python's `str.isspace` says it.
pub(crate) fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// `text` stripped of Python's white space at both ends.
pub(crate) fn py_strip(text: &str) -> &str {
    text.trim_matches(py_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_names_its_top_level_block_and_row() {
        assert_eq!(
            top_block_path("w:body/w:tbl[2]/w:tr[3]/w:tc[1]/w:p[1]"),
            Some("w:body/w:tbl[2]")
        );
        assert_eq!(
            top_block_path("w:body/w:sdt[1]/w:sdtContent[1]/w:p[4]"),
            Some("w:body/w:sdt[1]/w:sdtContent[1]/w:p[4]")
        );
        assert_eq!(
            row_of("w:body/w:tbl[2]/w:tr[3]/w:tc[1]/w:p[1]"),
            Some(("w:body/w:tbl[2]", 2))
        );
        // docx-agent's pattern names a content control's content without a count
        assert_eq!(
            row_of("w:body/w:sdt[1]/w:sdtContent[1]/w:tbl[1]/w:tr[1]/w:tc[1]/w:p[1]"),
            None
        );
        assert_eq!(
            row_of("w:body/w:sdt[1]/w:sdtContent/w:tbl[1]/w:tr[1]"),
            Some(("w:body/w:sdt[1]/w:sdtContent/w:tbl[1]", 0))
        );
        assert_eq!(row_of("w:body/w:p[1]"), None);
    }
}
