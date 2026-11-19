//! Owns: what keeping a span's blocks on one page writes, as docx-agent's
//! `keep_blocks_together` plans it (`_keep_changes`, `_keep_values`, `row_keeps`; measured
//! there, round 36): the body's blocks an address names ([`body_blocks`]: a span, a block,
//! a block-level control's blocks, or the top-level blocks a section's, bookmark's or
//! cell's paragraphs are in); every paragraph keeping its lines together and with the next
//! but the span's last; every table row keeping with the next by the paragraph Word
//! decides its keep by, but a span-ending table's last row -- each written directly only
//! where the styles do not give it (off: each direct keep the styles do not give
//! removed).
//!
//! Must not: write anything (the command writes the plan), or read outside the body.

use super::values::{Property, Setting, Value};
use crate::blocks::{blocks_of, resolve_block, BlockKind};
use crate::names::W;
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::address::{expand_heading, scope_paragraphs, span_ends};
use ooxml_word_model::docx::cells::{model_path, TableFacts};
use ooxml_word_model::docx::formatting::{Cell, EffectiveParagraph, Formatting};
use ooxml_word_model::docx::{at_path, Docx, StoryPart};
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// What keeping some blocks together writes.
#[derive(Debug, Default)]
pub(crate) struct KeepPlan {
    /// The first block's id.
    pub(crate) first: String,
    /// Each paragraph to write (its place in the main part) and what.
    pub(crate) changes: Vec<(Vec<u32>, Vec<Setting>)>,
    /// What the plan notes.
    pub(crate) warnings: Vec<String>,
}

fn refused(message: String) -> EditError {
    EditError::Refused(message)
}

fn is_w(element: &Element, local: &str) -> bool {
    element.name().is(Some(W), local)
}

/// The places of the body's blocks `address` names, in order (docx-agent's
/// `copies.blocks_of`), with the main part.
fn block_places(doc: &WordDoc, docx: &Docx, address: &str) -> Result<Vec<Vec<u32>>> {
    let expanded = expand_heading(docx, address).map_err(|e| refused(format!("{address}: {}", e.message)))?;
    let mut places: Vec<Vec<u32>> = Vec::new();
    let part;
    if span_ends(&expanded).map_err(EditError::Address)?.is_some() {
        let (found, blocks) = blocks_of(doc, docx, &expanded)?;
        part = found;
        places.extend(blocks.into_iter().map(|b| b.path));
    } else {
        let block = resolve_block(doc, docx, &expanded)
            .ok()
            .filter(|(_, b)| matches!(b.kind, BlockKind::Paragraph | BlockKind::Table | BlockKind::Control));
        if let Some((found, block)) = block {
            part = found;
            places.push(block.path);
        } else {
            let (story, indexes) =
                scope_paragraphs(docx, &expanded).map_err(|e| refused(format!("{address}: {}", e.message)))?;
            part = story.part.clone();
            for k in indexes {
                let Some(paragraph) = story.ids.paragraphs().get(k) else {
                    continue;
                };
                if let Some(top) = top_block(story.root(), &paragraph.path) {
                    if !places.contains(&top) {
                        places.push(top);
                    }
                }
            }
        }
    }
    if part != docx.main {
        return Err(refused(format!(
            "{address}: {address} is not in the body: a copy and its reference are the body's blocks"
        )));
    }
    Ok(places)
}

/// The body's top-level block holding the node at `path` (docx-agent's `_top_block`): the
/// outermost paragraph, table or control under the body, custom XML passed through.
fn top_block(root: &Arc<Element>, path: &[u32]) -> Option<Vec<u32>> {
    let (&body, rest) = path.split_first()?;
    let mut node = root.children().get(body as usize)?.as_element()?;
    if !is_w(node, "body") {
        return None;
    }
    let mut at = vec![body];
    for &k in rest {
        node = node.children().get(k as usize)?.as_element()?;
        at.push(k);
        if is_w(node, "p") || is_w(node, "tbl") || is_w(node, "sdt") {
            return Some(at);
        }
        if !is_w(node, "customXml") {
            return None;
        }
    }
    None
}

/// `(id, place)` of the body's blocks `address` names: a block-level control's blocks in
/// its place.
fn body_blocks(doc: &WordDoc, docx: &Docx, address: &str) -> Result<Vec<(String, Vec<u32>)>> {
    let body = docx.body().ok_or_else(|| EditError::NoPart(docx.main.clone()))?;
    let mut out = Vec::new();
    for place in block_places(doc, docx, address)? {
        let Some(element) = at_path(body.root(), &place) else {
            continue;
        };
        let mut nodes = Vec::new();
        if is_w(element, "sdt") {
            if let Some((k, content)) = child(element, "sdtContent") {
                for (j, node) in elements(content) {
                    if is_w(node, "p") || is_w(node, "tbl") {
                        nodes.push([place.as_slice(), &[k, j]].concat());
                    }
                }
            }
        } else {
            nodes.push(place);
        }
        for node in nodes {
            let id = body
                .ids
                .paragraph_at(&node)
                .map(|p| p.id)
                .or_else(|| body.ids.table_at(&node).map(|t| t.id));
            if let Some(id) = id {
                out.push((id, node));
            }
        }
    }
    Ok(out)
}

fn elements(parent: &Element) -> impl Iterator<Item = (u32, &Arc<Element>)> {
    parent.children().iter().enumerate().filter_map(|(k, n)| match n {
        Node::Element(e) => Some((u32::try_from(k).ok()?, e)),
        _ => None,
    })
}

fn child<'a>(parent: &'a Element, local: &str) -> Option<(u32, &'a Arc<Element>)> {
    elements(parent).find(|(_, e)| is_w(e, local))
}

/// The plan for keeping `address`'s blocks together (`on`) or releasing them.
///
/// # Errors
/// docx-agent's refusals: an address naming nothing in the body, or no blocks.
pub(crate) fn keep_plan(doc: &WordDoc, docx: &Docx, address: &str, on: bool) -> Result<KeepPlan> {
    let blocks = body_blocks(doc, docx, address)?;
    let Some((first, _)) = blocks.first() else {
        return Err(refused(format!(
            "{address} holds no paragraph or table to keep together"
        )));
    };
    let body = docx.body().ok_or_else(|| EditError::NoPart(docx.main.clone()))?;
    let mut look = Keeps::new(docx, body);
    let mut plan = KeepPlan {
        first: first.clone(),
        ..KeepPlan::default()
    };
    let last = blocks.len() - 1;
    for (k, (id, place)) in blocks.iter().enumerate() {
        let Some(element) = at_path(body.root(), place) else {
            continue;
        };
        if is_w(element, "p") {
            let wanted: &[Property] = if k < last {
                &[Property::KeepTogether, Property::KeepWithNext]
            } else {
                &[Property::KeepTogether]
            };
            let values = look.values(element, place, wanted, on);
            if !values.is_empty() {
                plan.changes.push((place.clone(), values));
            }
            continue;
        }
        let rows = row_leads(element, place);
        for (r, lead) in rows.iter().enumerate() {
            if k == last && r + 1 == rows.len() {
                continue; // the span's end keeps with what follows as it did
            }
            let Some(lead) = lead else {
                let note = format!("{id} has a row with no paragraph to keep it by: that row may part");
                if !plan.warnings.contains(&note) {
                    plan.warnings.push(note);
                }
                continue;
            };
            let Some(paragraph) = at_path(body.root(), lead) else {
                continue;
            };
            let keeps = look.effective(paragraph, lead, true).keep_with_next;
            let styled = look.effective(paragraph, lead, false).keep_with_next;
            let declared = declared(paragraph, "keepNext") == Some(true);
            let value = if on && !keeps {
                Some((!styled).then_some(Value::Flag(true)))
            } else if !on && declared && !styled {
                Some(None)
            } else {
                None
            };
            if let Some(value) = value {
                let setting = Setting {
                    property: Property::KeepWithNext,
                    value,
                };
                plan.changes.push((lead.clone(), vec![setting]));
            }
        }
    }
    Ok(plan)
}

/// A paragraph's direct toggle `w:{local}`: on, off, or not said.
fn declared(paragraph: &Element, local: &str) -> Option<bool> {
    let node = paragraph.find(Some(W), "pPr")?.find(Some(W), local)?;
    Some(
        node.attribute(Some(W), "val")
            .is_none_or(|v| !matches!(v.to_lowercase().as_str(), "0" | "false" | "off")),
    )
}

/// Per row of a table, the place of the paragraph Word decides the row's keep with the next
/// by (docx2svg's `lead_paragraph`: its first cell's first paragraph, a nested table's
/// first there).
fn row_leads(table: &Element, place: &[u32]) -> Vec<Option<Vec<u32>>> {
    let mut out = Vec::new();
    for (k, row) in rows(table) {
        let at = [place, k.as_slice()].concat();
        let cell = contained(row, "tc").into_iter().next();
        out.push(cell.and_then(|(j, tc)| first_paragraph(tc, &[at.as_slice(), j.as_slice()].concat())));
    }
    out
}

/// `parent`'s `w:{local}` children, through content controls and custom XML, each with its
/// steps from `parent`.
fn contained<'a>(parent: &'a Element, local: &str) -> Vec<(Vec<u32>, &'a Arc<Element>)> {
    let mut out = Vec::new();
    for (k, node) in elements(parent) {
        if is_w(node, local) {
            out.push((vec![k], node));
        } else if is_w(node, "customXml") {
            out.extend(
                contained(node, local)
                    .into_iter()
                    .map(|(p, n)| ([&[k][..], &p].concat(), n)),
            );
        } else if is_w(node, "sdt") {
            if let Some((j, content)) = child(node, "sdtContent") {
                out.extend(
                    contained(content, local)
                        .into_iter()
                        .map(|(p, n)| ([&[k, j][..], &p].concat(), n)),
                );
            }
        }
    }
    out
}

/// A table's rows as the layout reads them: a row deleted as a tracked change is gone.
fn rows(table: &Element) -> Vec<(Vec<u32>, &Arc<Element>)> {
    let deleted = |row: &Element| {
        row.find(Some(W), "trPr")
            .is_some_and(|p| p.find(Some(W), "del").is_some())
    };
    contained(table, "tr")
        .into_iter()
        .filter(|(_, r)| !deleted(r))
        .collect()
}

fn first_paragraph(container: &Element, place: &[u32]) -> Option<Vec<u32>> {
    let (k, node) = elements(container).find(|(_, e)| is_w(e, "p") || is_w(e, "tbl") || is_w(e, "sdt"))?;
    let at = [place, &[k]].concat();
    if is_w(node, "p") {
        return Some(at);
    }
    if is_w(node, "sdt") {
        let (j, content) = child(node, "sdtContent")?;
        return first_paragraph(content, &[at.as_slice(), &[j]].concat());
    }
    let (r, row) = rows(node).into_iter().next()?;
    let row_at = [at.as_slice(), r.as_slice()].concat();
    let (c, cell) = contained(row, "tc").into_iter().next()?;
    first_paragraph(cell, &[row_at.as_slice(), c.as_slice()].concat())
}

/// Paragraphs' keeps as Word applies them, with and without their direct formatting.
struct Keeps<'a> {
    formatting: Formatting,
    body: &'a StoryPart,
    tables: HashMap<Vec<u32>, Arc<TableFacts>>,
}

impl<'a> Keeps<'a> {
    fn new(docx: &Docx, body: &'a StoryPart) -> Keeps<'a> {
        Keeps {
            formatting: Formatting::read(docx),
            body,
            tables: HashMap::new(),
        }
    }

    /// The cell of the outermost table holding the paragraph at `place`.
    fn cell(&mut self, place: &[u32]) -> Option<Cell> {
        let root = self.body.root();
        let depth = (1..place.len())
            .find(|d| at_path(root, place.get(..*d).unwrap_or_default()).is_some_and(|e| is_w(e, "tbl")))?;
        let (table_at, below) = place.split_at(depth);
        let table = at_path(root, table_at)?;
        let facts = self
            .tables
            .entry(table_at.to_vec())
            .or_insert_with(|| Arc::new(TableFacts::read(&self.formatting, table)));
        facts.cell(&model_path(table, below)?).cloned()
    }

    /// The paragraph's formatting as Word applies it; without `direct`, what its styles
    /// (and its list) give it.
    fn effective(&mut self, paragraph: &Element, place: &[u32], direct: bool) -> EffectiveParagraph {
        let cell = self.cell(place);
        if direct {
            return self.formatting.paragraph(paragraph, cell.as_ref());
        }
        let styled = styled_only(paragraph);
        self.formatting.paragraph(&styled, cell.as_ref())
    }

    /// docx-agent's `_keep_values`.
    fn values(&mut self, paragraph: &Element, place: &[u32], names: &[Property], on: bool) -> Vec<Setting> {
        let applied = self.effective(paragraph, place, true);
        let styled = self.effective(paragraph, place, false);
        let pick = |e: &EffectiveParagraph, p: Property| match p {
            Property::KeepTogether => e.keep_together,
            _ => e.keep_with_next,
        };
        let mut out = Vec::new();
        for &name in names {
            let local = if name == Property::KeepTogether {
                "keepLines"
            } else {
                "keepNext"
            };
            let value = if on && !pick(&applied, name) {
                Some((!pick(&styled, name)).then_some(Value::Flag(true)))
            } else if !on && declared(paragraph, local) == Some(true) && !pick(&styled, name) {
                Some(None)
            } else {
                None
            };
            if let Some(value) = value {
                out.push(Setting { property: name, value });
            }
        }
        out
    }
}

/// A copy of `paragraph` whose properties are only its style and its list.
fn styled_only(paragraph: &Element) -> Element {
    let mut work = crate::work::Work::of(&Arc::new(paragraph.clone()), Vec::new());
    let root = work.root();
    if let Some(properties) = work.find(root, W, "pPr") {
        let children: Vec<_> = work.children(properties).to_vec();
        for c in children {
            if !work.is_any(c, W, &["pStyle", "numPr"]) {
                work.remove(c);
            }
        }
    }
    work.build().map_or_else(|| paragraph.clone(), |e| (*e).clone())
}
