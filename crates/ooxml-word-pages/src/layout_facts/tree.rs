//! Owns: what the layout facts read of the document's tree: the body's top-level blocks
//! with their keys, layout paths, places and ids ([`top_level`], docx-agent's
//! `_top_level`), where a body paragraph is among the body's blocks by key (its top-level
//! block, its tables, the body table's row and cell it is in: [`In`], docx-agent's
//! `_ROW` and `_CELL_LINE`), a layout path's element in its part, a paragraph's style, and
//! the paragraph tests the facts share (`_page_break_at`, `_CONTENT`).
//!
//! Must not: read the layout.

use crate::line_text::TableRow;
use ooxml_word_model::docx::{Docx, StoryPart, VersionNames};
use ooxml_word_model::ids::{Anchor, BlockKey, BlockKind as KeyKind, Entry, PartIds, StoryIx, Sub};
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

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
    /// Its key.
    pub key: Option<BlockKey>,
    /// The ids that name it, and its entry among them.
    ids: &'a PartIds,
    entry: Entry,
    /// Its id, named when first asked.
    id: OnceLock<String>,
}

impl<'a> Block<'a> {
    /// The block `entry` of `ids` names, at `place` (its layout path `path`).
    pub(crate) fn of(
        kind: BlockKind,
        element: &'a Arc<Element>,
        place: Vec<u32>,
        path: String,
        ids: &'a PartIds,
        entry: Entry,
    ) -> Block<'a> {
        Block {
            kind,
            element,
            place,
            path,
            key: ids.key_of(StoryIx::BODY, entry),
            ids,
            entry,
            id: OnceLock::new(),
        }
    }

    /// The block, named `id` (where it was asked for by an id its entry is found by).
    pub(crate) fn named(self, id: &str) -> Block<'a> {
        let _ = self.id.set(id.to_owned());
        self
    }

    /// Its entry among the ids.
    pub(crate) fn entry(&self) -> Entry {
        self.entry
    }

    /// Its id, named when first asked: a check names only the blocks it reports.
    #[must_use]
    pub fn id(&self) -> &str {
        self.id
            .get_or_init(|| self.ids.id_of(self.entry).unwrap_or_default())
            .as_str()
    }
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
            // Named where the ids hold it (`id_of` answers for every entry `at` finds).
            if let Some(entry) = story.ids.at(place) {
                let kind = if is_w(child, "p") {
                    BlockKind::Paragraph
                } else {
                    BlockKind::Table
                };
                out.push(Block::of(kind, child, place.clone(), here, &story.ids, entry));
            }
        }
        place.pop();
    }
}

/// Where a body paragraph is among the body's blocks, read by key through the version's
/// names: what docx-agent reads of a line's path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct In {
    /// The blocks it is in, outermost first, ending with it ([`VersionNames::blocks_of`]).
    chain: Vec<Anchor>,
    /// The outermost table's row it is in, where that table is right in the body
    /// (`_ROW`: docx-agent's pattern reads no table in a content control or custom XML).
    pub(crate) row: Option<TableRow>,
    /// Whether it is right in that row's cell, not in a nested table or a wrapper
    /// (`_CELL_LINE`).
    pub(crate) in_cell: bool,
}

impl In {
    /// Where the body paragraph `paragraph` is; `None` where it is not a body paragraph
    /// of the version.
    pub(crate) fn of(names: &VersionNames, paragraph: BlockKey) -> Option<In> {
        if paragraph.story() != StoryIx::BODY {
            return None;
        }
        let chain = names.blocks_of(paragraph)?;
        let part = names.part(StoryIx::BODY)?;
        let container = part.ids.container().len();
        let table = chain
            .first()
            .filter(|a| a.sub == Sub::Whole && a.block.kind() == KeyKind::Table)
            .filter(|a| names.place_of(a.block).is_some_and(|p| p.len() == container + 1));
        let row = table.and_then(|t| match chain.get(2)?.sub {
            Sub::Cell(row, _) => Some(TableRow {
                table: t.block,
                row: usize::from(row),
            }),
            _ => None,
        });
        let in_cell = row.is_some()
            && chain.len() == 4
            && match (table.and_then(|t| names.place_of(t.block)), names.place_of(paragraph)) {
                (Some(table), Some(own)) => right_in_cell(part, &table, &own),
                _ => false,
            };
        Some(In { chain, row, in_cell })
    }

    /// Its top-level block (a paragraph or table: itself where it is one).
    pub(crate) fn top(&self) -> Option<BlockKey> {
        self.chain.first().map(|a| a.block)
    }

    /// Every table it is in, outermost first.
    pub(crate) fn tables(&self) -> impl Iterator<Item = BlockKey> + '_ {
        self.chain
            .iter()
            .filter(|a| a.sub == Sub::Whole && a.block.kind() == KeyKind::Table)
            .map(|a| a.block)
    }
}

/// Whether the paragraph at `own` (a place in `part`) is right in its cell's `w:tc`: its
/// parent is the `w:tc`, `table` the place of the table it is a cell paragraph of.
fn right_in_cell(part: &StoryPart, table: &[u32], own: &[u32]) -> bool {
    let Some((_, parent)) = own.split_last() else {
        return false;
    };
    parent.starts_with(table) && ooxml_word_model::docx::at_path(part.root(), parent).is_some_and(|e| is_w(e, "tc"))
}

/// The top-level block of the body's (a paragraph or table below the body and its
/// block-level content controls and custom XML) the body block `key` is in, itself where it
/// is one -- docx2svg's `block_keys(key)[0]`.
pub(crate) fn top_of(names: &VersionNames, key: BlockKey) -> Option<BlockKey> {
    if key.story() != StoryIx::BODY {
        return None;
    }
    let part = names.part(StoryIx::BODY)?;
    let place = names.place_of(key)?;
    let container = part.ids.container();
    let mut node = ooxml_word_model::docx::at_path(part.root(), container)?;
    for depth in container.len()..place.len() {
        node = node.children().get(*place.get(depth)? as usize)?.as_element()?;
        if is_w(node, "p") || is_w(node, "tbl") {
            return part.ids.key_at(StoryIx::BODY, place.get(..=depth)?);
        }
    }
    None
}

/// Whether `anchor` is a top-level block of the body's ([`top_of`]), as docx2svg's
/// `block_keys(key)[0] == key` reads a page's block.
pub(crate) fn is_top(names: &VersionNames, anchor: Anchor) -> bool {
    anchor.sub == Sub::Whole && anchor.block.kind() != KeyKind::Row && top_of(names, anchor.block) == Some(anchor.block)
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

/// A line of a paragraph right in a body table's cell (not a nested table's), as
/// docx-agent's `_CELL_LINE` reads it: the table's path (as [`row_of`] reads it), the row
/// and the cell (`w:tc`, among the row's own children) from 0, and the paragraph's steps
/// below the cell (`w:p[n]`, or `w:sdt[n]/w:sdtContent/w:p[n]`).
pub(crate) fn cell_line(path: &str) -> Option<(&str, usize, usize, &str)> {
    let (table, row) = row_of(path)?;
    let rest = path.get(table.len()..)?.strip_prefix('/')?;
    let (_, rest) = rest.split_once('/')?;
    let (cell, paragraph) = rest.split_once('/')?;
    let cell = indexed(cell, "w:tc")?.checked_sub(1)?;
    let steps: Vec<&str> = paragraph.split('/').collect();
    let plain = match steps.as_slice() {
        [p] => indexed(p, "w:p").is_some(),
        [sdt, "w:sdtContent", p] => indexed(sdt, "w:sdt").is_some() && indexed(p, "w:p").is_some(),
        _ => false,
    };
    plain.then_some((table, row, cell, paragraph))
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
/// root), by docx2svg's path rules (`ooxml_word_model::read::paths`): a text box's
/// paragraphs, rows in content controls and notes by id too.
pub(crate) fn resolve<'a>(story: &'a StoryPart, path: &str) -> Option<(&'a Arc<Element>, Vec<u32>)> {
    story.resolve_path(path)
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

/// Every paragraph below `element` (nested ones too, `element` itself where it is one), in
/// document order, each with its place (`place` being `element`'s).
pub(crate) fn paragraphs_below<'e>(element: &'e Arc<Element>, place: &[u32]) -> Vec<(&'e Arc<Element>, Vec<u32>)> {
    let mut out = Vec::new();
    let mut stack: Vec<(&Arc<Element>, Vec<u32>)> = vec![(element, place.to_vec())];
    while let Some((node, at)) = stack.pop() {
        if is_w(node, "p") {
            out.push((node, at.clone()));
        }
        let below: Vec<(&Arc<Element>, Vec<u32>)> = node
            .children()
            .iter()
            .enumerate()
            .filter_map(|(k, n)| {
                let child = n.as_element()?;
                let mut here = at.clone();
                here.push(u32::try_from(k).unwrap_or(u32::MAX));
                Some((child, here))
            })
            .collect();
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
    let found = if start { content.next() } else { content.next_back() };
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
    fn a_painted_line_names_its_row_and_cell() {
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
        assert_eq!(
            cell_line("w:body/w:tbl[2]/w:tr[3]/w:tc[1]/w:p[4]"),
            Some(("w:body/w:tbl[2]", 2, 0, "w:p[4]"))
        );
        assert_eq!(
            cell_line("w:body/w:tbl[1]/w:tr[1]/w:tc[2]/w:sdt[1]/w:sdtContent/w:p[1]"),
            Some(("w:body/w:tbl[1]", 0, 1, "w:sdt[1]/w:sdtContent/w:p[1]"))
        );
        // a nested table's paragraph is not its outer cell's
        assert_eq!(
            cell_line("w:body/w:tbl[1]/w:tr[1]/w:tc[1]/w:tbl[1]/w:tr[1]/w:tc[1]/w:p[1]"),
            None
        );
    }
}
