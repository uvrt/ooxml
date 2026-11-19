//! Owns: `word_compare_blocks` (docx-agent's `edit/compare.py` and its tool in
//! `tools/style.py`): each block compared with a reference block, in this document or in
//! another open document the call names (`reference.doc`, handed over by the session at the
//! version pinned for the call), every difference listed with the reference's value and
//! the calls that would give it, paged by difference; and what a comparison reads of a
//! block and of a document ([`Side`], [`Block`]).
//!
//! The tables' properties are [`super::compare_tables`]'s, the paragraphs' and how a cell's
//! paragraphs pair [`super::compare_paragraphs`]'s.
//!
//! Must not: change a document, or judge whether a difference is intended.

use super::compare_args::WordCompareBlocks;
use super::compare_paragraphs::paragraphs;
use super::compare_tables::tables;
use crate::paging::page_list;
use crate::refusal::address_refused;
use crate::version::Version;
use ooxml_base::Cancel;
use ooxml_tools::{Done, Refusal, ToolError, TryCall, WayOut};
use ooxml_word_model::address::{resolve, AddressError};
use ooxml_word_model::docx::formatting::{Cell, Formatting};
use ooxml_word_model::docx::{style_name, Docx, StoryPart};
use ooxml_word_model::ids::{Entry, W};
use ooxml_word_model::markdown::read::{table_style, Placed, Reader};
use ooxml_word_model::markdown::ViewName;
use ooxml_xml::Element;
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// Every property compared, in the order reported.
pub(crate) const PROPERTIES: [&str; 36] = [
    "kind",
    "header_rows",
    "columns",
    "rows",
    "merges",
    "widths",
    "width",
    "table_style",
    "table_alignment",
    "table_shading",
    "table_borders",
    "shading",
    "borders",
    "vertical_alignment",
    "paragraphs",
    "missing_paragraph",
    "extra_paragraph",
    "paragraph_style",
    "alignment",
    "space_before",
    "space_after",
    "line_spacing",
    "indent_left",
    "indent_right",
    "indent_first",
    "keep_with_next",
    "keep_lines",
    "page_break_before",
    "bold",
    "italic",
    "underline",
    "strike",
    "size",
    "font",
    "color",
    "highlight",
];

/// Differences in one page of a comparison.
const PAGE: usize = 60;

/// One difference, its keys in the order reported (`block` put first when listed).
pub(crate) type Difference = Map<String, Value>;

/// One document as a comparison reads it: its version's reading and formatting.
pub(crate) struct Side<'d> {
    /// The version's reading.
    pub(crate) docx: &'d Docx,
    reader: Reader<'d>,
}

impl<'d> Side<'d> {
    /// The document `docx` as compared.
    pub(crate) fn new(docx: &'d Docx) -> Self {
        Side {
            docx,
            reader: Reader::new(docx, ViewName::Final),
        }
    }

    /// What formatting resolves against.
    pub(crate) fn formatting(&self) -> &Formatting {
        self.reader.shared().formatting()
    }

    /// Where the paragraph at `path` of `part` stands in a table (`None`: in none), as
    /// docx-agent's effective formatting reads it.
    pub(crate) fn cell(&self, part: &StoryPart, element: &Arc<Element>, path: &[u32]) -> Option<Cell> {
        let style = (1..path.len())
            .rev()
            .find_map(|n| path.get(..n).and_then(|prefix| part.ids.table_at(prefix)))
            .map(|table| table_style(&table.element))?;
        let placed = Placed {
            element: Arc::clone(element),
            path: path.into(),
        };
        self.reader.cell_of(&part.part, &placed, &style)
    }

    /// A style's name by its id (docx-agent's `_style_name`): the id itself where no style
    /// has it.
    pub(crate) fn style_name(&self, id: Option<&str>) -> Option<String> {
        let id = id?;
        match self.docx.style(id) {
            Some(style) => style_name(style).map(str::to_owned),
            None => Some(id.to_owned()),
        }
    }

    /// The default paragraph style's id.
    pub(crate) fn default_style(&self) -> Option<&str> {
        self.docx
            .default_style("paragraph")
            .and_then(|s| s.attribute(Some(W), "styleId"))
    }

    /// Whether the document has, or would add, the `kind` style `name`.
    pub(crate) fn has_style(&self, name: &str, kind: &str) -> bool {
        ooxml_word_edit::known_style(self.docx, name, kind).is_ok()
    }
}

/// A block read for comparing: its id, part and element, and its place there.
pub(crate) struct Block<'d> {
    /// Its id.
    pub(crate) id: String,
    /// Its story part.
    pub(crate) part: &'d StoryPart,
    /// The element.
    pub(crate) element: Arc<Element>,
    /// Its place in the part.
    pub(crate) path: Vec<u32>,
    /// A table (else a paragraph).
    pub(crate) table: bool,
}

impl<'d> Block<'d> {
    /// The block `id` names in `docx`.
    pub(crate) fn of(docx: &'d Docx, id: &str) -> Result<Block<'d>, AddressError> {
        let (part, entry) = resolve(docx, id)?;
        let found = match entry {
            Entry::Table(k) => part
                .ids
                .tables()
                .get(k)
                .map(|t| (t.id, t.element, t.path.to_vec(), true)),
            Entry::Paragraph(k) => part
                .ids
                .paragraphs()
                .get(k)
                .map(|p| (p.id, p.element, p.path.to_vec(), false)),
        };
        let (id, element, path, table) = found.ok_or_else(|| ooxml_word_model::address::resolve::unknown(docx, id))?;
        Ok(Block {
            id,
            part,
            element,
            path,
            table,
        })
    }

    /// The id of the paragraph at `below` (its place below the block), where it has one.
    pub(crate) fn paragraph_id(&self, below: &[u32]) -> Option<String> {
        let path: Vec<u32> = self.path.iter().chain(below).copied().collect();
        self.part.ids.paragraph_at(&path).map(|p| p.id)
    }

    fn kind(&self) -> &'static str {
        if self.table {
            "table"
        } else {
            "paragraph"
        }
    }
}

/// A paragraph to compare: the element, its place below its block (empty: the block).
pub(crate) type Para = (Arc<Element>, Vec<u32>);

/// Each of `blocks` (of `mine`) compared with `reference` (a block of `theirs`):
/// `{"reference", "compared", "same", "differences"}`, every difference with its block.
///
/// # Errors
/// The address resolver's refusal of the reference or of a block.
pub(crate) fn compare(
    mine: &Side<'_>,
    blocks: &[String],
    reference: &str,
    theirs: &Side<'_>,
) -> Result<Map<String, Value>, AddressError> {
    let source = Block::of(theirs.docx, reference)?;
    let (mut out, mut same, mut compared) = (Vec::new(), Vec::new(), Vec::new());
    for identifier in blocks {
        let block = Block::of(mine.docx, identifier)?;
        compared.push(Value::from(block.id.clone()));
        let mut found: Vec<Difference> = Vec::new();
        if block.table != source.table {
            found.push(difference(
                "kind",
                None,
                json!(block.kind()),
                json!(source.kind()),
                Value::Null,
            ));
        } else if block.table {
            found = tables(&block, mine, &source, theirs);
        } else {
            let pair = vec![(
                (Arc::clone(&block.element), Vec::new()),
                (Arc::clone(&source.element), Vec::new()),
            )];
            found = paragraphs(&block, mine, &source, theirs, &pair);
        }
        if found.is_empty() {
            same.push(Value::from(block.id.clone()));
        }
        let order = |d: &Difference| {
            let property = d.get("property").and_then(Value::as_str).unwrap_or("");
            PROPERTIES
                .iter()
                .position(|p| *p == property)
                .unwrap_or(PROPERTIES.len())
        };
        found.sort_by_key(order);
        for entry in found {
            let mut listed = Map::new();
            listed.insert("block".into(), Value::from(block.id.clone()));
            listed.extend(entry);
            out.push(Value::Object(listed));
        }
    }
    let mut data = Map::new();
    data.insert("reference".into(), Value::from(source.id));
    data.insert("compared".into(), Value::Array(compared));
    data.insert("same".into(), Value::Array(same));
    data.insert("differences".into(), Value::Array(out));
    Ok(data)
}

/// A difference as listed: its property, where it is (`cells`, `rows`, `paragraphs`), its
/// value, the reference's and the fix.
pub(crate) fn difference(
    property: &str,
    at: Option<(&str, Value)>,
    value: Value,
    reference: Value,
    fix: Value,
) -> Difference {
    let mut out = Map::new();
    out.insert("property".into(), Value::from(property));
    if let Some((key, places)) = at {
        out.insert(key.into(), places);
    }
    out.insert("value".into(), value);
    out.insert("reference".into(), reference);
    out.insert("fix".into(), fix);
    out
}

/// Values grouped by key, the keys in the order first met (a Python dict of lists).
pub(crate) struct Groups<K, V>(Vec<(K, Vec<V>)>);

impl<K: PartialEq, V> Groups<K, V> {
    /// No groups.
    pub(crate) fn new() -> Self {
        Groups(Vec::new())
    }

    /// `value` added to `key`'s group.
    pub(crate) fn add(&mut self, key: K, value: V) {
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some((_, values)) => values.push(value),
            None => self.0.push((key, vec![value])),
        }
    }
}

impl<K, V> IntoIterator for Groups<K, V> {
    type Item = (K, Vec<V>);
    type IntoIter = std::vec::IntoIter<(K, Vec<V>)>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// `{"tool": tool, "calls": calls}`.
pub(crate) fn fix(tool: &str, calls: Vec<Value>) -> Value {
    json!({"tool": tool, "calls": calls})
}

/// `word_compare_blocks` on a Word document at one version, its reference in it or in
/// another open document the call names.
///
/// # Errors
/// The address resolver's refusal of a block or of the reference; `not_found` for a
/// reference document the call was not handed; `invalid_arguments` for a cursor no page
/// gave; `timeout` when cancelled.
pub fn word_compare_blocks(
    args: &WordCompareBlocks,
    version: &Version<'_>,
    cancel: &dyn Cancel,
) -> Result<Done<Value>, ToolError> {
    let other = args
        .reference
        .doc
        .as_deref()
        .filter(|d| !d.is_empty() && *d != args.doc);
    let docx = version.docx()?;
    let reference = match other {
        Some(id) => version.others.get(id).ok_or_else(|| not_open(id, &args.doc))?.docx()?,
        None => Arc::clone(&docx),
    };
    let (mine, theirs) = (Side::new(&docx), Side::new(&reference));
    let mut first = args.clone();
    first.cursor = None;
    let mut data = compare(&mine, &args.blocks, &args.reference.block, &theirs)
        .map_err(|e| address_refused(e, try_call(&first)))?;
    cancel.check().map_err(crate::describe::cancelled)?;
    let all = match data.remove("differences") {
        Some(Value::Array(all)) => all,
        _ => Vec::new(),
    };
    let (page, total, next) = page_list(&all, args.cursor.as_deref(), PAGE, try_call(&first))?;
    let fixes = page
        .iter()
        .filter(|d| d.get("fix").is_some_and(|f| !f.is_null()))
        .count();
    let count = |key: &str| data.get(key).and_then(Value::as_array).map_or(0, Vec::len);
    let summary = format!(
        "Compared {} block(s) with {}: {} the same, {total} difference(s) ({fixes} with a fix on this page)",
        count("compared"),
        data.get("reference").and_then(Value::as_str).unwrap_or(""),
        count("same"),
    );
    data.insert("differences".into(), Value::Array(page));
    let mut done = Done::new(summary);
    done.data = Some(Value::Object(data));
    done.total = Some(total as u64);
    done.next_cursor = next;
    Ok(done)
}

fn try_call(args: &WordCompareBlocks) -> TryCall {
    let mut reference = Map::new();
    if let Some(doc) = &args.reference.doc {
        reference.insert("doc".into(), Value::from(doc.as_str()));
    }
    reference.insert("block".into(), Value::from(args.reference.block.as_str()));
    let mut arguments = Map::new();
    arguments.insert("doc".into(), Value::from(args.doc.as_str()));
    arguments.insert("blocks".into(), json!(args.blocks));
    arguments.insert("reference".into(), Value::Object(reference));
    if let Some(cursor) = &args.cursor {
        arguments.insert("cursor".into(), Value::from(cursor.as_str()));
    }
    TryCall {
        tool: "word_compare_blocks".into(),
        arguments,
        fill: Vec::new(),
    }
}

/// A reference document the call was not handed (the session refuses one not open first).
fn not_open(id: &str, doc: &str) -> ToolError {
    let message = format!("no open document {}", ooxml_tools::pyfmt::repr_str(id));
    match WayOut::names([doc.to_owned()]) {
        Some(way_out) => ToolError::refusal(Refusal::NotFound, message, way_out).at("reference.doc"),
        None => ToolError::internal(message, "word_compare_blocks:reference"),
    }
}
