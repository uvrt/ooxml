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
//! Also owns what a check asks of a range's keep ([`kept_blocks`], [`keep_writes`]) and the
//! package a trial lays out ([`kept_package`]: the plan written on a copy of the main part,
//! nothing else changed).
//!
//! Must not: change the document (the command writes the plan), or read outside the body.

use super::values::{write_paragraph, Property, Setting, Value};
use crate::blocks::{blocks_of, resolve_block, BlockKind};
use crate::names::W;
use crate::work::{scope_above, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::address::Address;
use ooxml_word_model::address::{expand_heading, scope_paragraphs};
use ooxml_word_model::docx::formatting::{EffectiveParagraph, Formatting};
use ooxml_word_model::docx::keeps::{declared, row_leads, Keeps};
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::{Element, Node, NodePath, Patch};
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
fn block_places(doc: &WordDoc, docx: &Docx, address: &Address) -> Result<Vec<Vec<u32>>> {
    let expanded = expand_heading(docx, address).map_err(|e| refused(format!("{address}: {}", e.message)))?;
    let mut places: Vec<Vec<u32>> = Vec::new();
    let part;
    if expanded.span_ends().map_err(EditError::Address)?.is_some() {
        let (found, blocks) = blocks_of(doc, docx, &expanded)?;
        part = found;
        places.extend(blocks.into_iter().map(|b| b.path));
    } else {
        let block = resolve_block(doc, docx, expanded.as_str())
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

/// `(id, place)` of the body's blocks `address` names, in order (docx-agent's
/// `copies.blocks_of`): a block-level control's blocks in its place.
///
/// # Errors
/// An address naming nothing in the body, or not in the body.
pub fn body_blocks(doc: &WordDoc, docx: &Docx, address: &Address) -> Result<Vec<(String, Vec<u32>)>> {
    let body = docx.body().ok_or_else(|| EditError::no_part(docx.main.clone()))?;
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
pub(crate) fn keep_plan(doc: &WordDoc, docx: &Docx, address: &Address, on: bool) -> Result<KeepPlan> {
    let blocks = body_blocks(doc, docx, address)?;
    let Some((first, _)) = blocks.first() else {
        return Err(refused(format!(
            "{address} holds no paragraph or table to keep together"
        )));
    };
    let body = docx.body().ok_or_else(|| EditError::no_part(docx.main.clone()))?;
    let formatting = Formatting::read(docx);
    let mut look = Keeps::new(docx, &formatting, body);
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
            let values = keep_values(&mut look, element, place, wanted, on);
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

/// docx-agent's `_keep_values`: what keeping (`on`) or releasing writes on the paragraph at
/// `place` for each of `names`, each written directly only where the styles do not give it.
fn keep_values(look: &mut Keeps<'_>, paragraph: &Element, place: &[u32], names: &[Property], on: bool) -> Vec<Setting> {
    let applied = look.effective(paragraph, place, true);
    let styled = look.effective(paragraph, place, false);
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

/// The ids of the body's blocks `address` names, in order (docx-agent's
/// `copies.blocks_of`): those a keep of it writes on, a block-level control's blocks in its
/// place.
///
/// # Errors
/// An address naming nothing in the body.
pub fn kept_blocks(doc: &WordDoc, docx: &Docx, address: &Address) -> Result<Vec<String>> {
    Ok(body_blocks(doc, docx, address)?.into_iter().map(|(id, _)| id).collect())
}

/// Whether keeping `address`'s blocks together (`on`) or releasing them writes anything:
/// `false` where they are kept so (or free) already.
///
/// # Errors
/// As [`keep_plan`]'s.
pub fn keep_writes(doc: &WordDoc, docx: &Docx, address: &Address, on: bool) -> Result<bool> {
    Ok(!keep_plan(doc, docx, address, on)?.changes.is_empty())
}

/// The document's package with only what keeping (`on`) or releasing `address`'s blocks
/// writes -- `word_format` `keep_together`'s paragraph settings, untracked and unstamped
/// (docx-agent's `keep_trials._changed`): what a check lays out to say what the call would
/// do. `None` where it writes nothing.
///
/// # Errors
/// As [`keep_plan`]'s; the main part not XML.
pub fn kept_package(
    doc: &WordDoc,
    docx: &Docx,
    address: &Address,
    on: bool,
) -> Result<Option<ooxml_word_model::WordPackage>> {
    let plan = keep_plan(doc, docx, address, on)?;
    if plan.changes.is_empty() {
        return Ok(None);
    }
    let main = docx.main.clone();
    let mut tree = doc.tree(&main)?.ok_or_else(|| EditError::no_part(main.clone()))?;
    let shared = doc.shared_handle();
    let orders = shared.facts.orders();
    for (place, settings) in &plan.changes {
        let (&last, above) = place.split_last().ok_or_else(|| EditError::no_element(main.clone()))?;
        let element = at_path(tree.root(), place).ok_or_else(|| EditError::no_element(main.clone()))?;
        let mut work = Work::of(element, scope_above(tree.root(), place));
        let root = work.root();
        write_paragraph(&mut work, root, settings, orders);
        let new = work.build().ok_or_else(|| EditError::unbuilt(main.clone()))?;
        let old = Node::Element(Arc::clone(element));
        let mut parent = vec![0usize];
        parent.extend(above.iter().map(|k| *k as usize));
        let patch = Patch::splice(NodePath(parent), last as usize, vec![old], vec![Node::Element(new)]);
        tree = patch.apply(&tree).map_err(|_| EditError::no_element(main.clone()))?;
    }
    Ok(Some(ooxml_word_model::WordPackage::edited(
        doc.package().with_new_tree(&main, tree),
    )))
}
