//! Owns: the blocks an address names for the block tools, as docx-agent's `_blocks_of`
//! reads one: a heading's range expanded (`h:p:A`), a span of blocks of one container
//! (`p:A..t:B`, either end open: `_span_elements`), or one block -- a paragraph, a table
//! or a block-level content control (`_resolve_block`) -- each with its place.
//!
//! Must not: change anything.

use crate::set_text::find;
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::address::{expand_heading, resolve, span_ends, AddressError, SPAN_SYNTAX};
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_word_model::ids::{Entry, W};
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// What a block is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockKind {
    /// A paragraph.
    Paragraph,
    /// A table.
    Table,
    /// A block-level content control.
    Control,
    /// A block-level custom XML element.
    Custom,
}

/// A block and its place.
#[derive(Debug, Clone)]
pub(crate) struct BlockAt {
    /// What it is.
    pub(crate) kind: BlockKind,
    /// Its place in its part.
    pub(crate) path: Vec<u32>,
}

/// What a block element is, if it is one a span holds (`_MOVABLE`).
pub(crate) fn kind_of(element: &Element) -> Option<BlockKind> {
    let name = element.name();
    if name.namespace() != Some(W) {
        return None;
    }
    Some(match name.local() {
        "p" => BlockKind::Paragraph,
        "tbl" => BlockKind::Table,
        "sdt" => BlockKind::Control,
        "customXml" => BlockKind::Custom,
        _ => return None,
    })
}

/// The block `id` names: a paragraph or table by id, or a block-level content control
/// (`cc:`, `cc@`), with its part.
///
/// # Errors
/// The address's refusal; a control that is not a block.
pub(crate) fn resolve_block(doc: &WordDoc, docx: &Docx, id: &str) -> Result<(String, BlockAt)> {
    if id.starts_with("cc:") || id.starts_with("cc@") {
        let found = controls(docx).into_iter().find(|c| c.id == id).ok_or_else(|| {
            EditError::Address(AddressError::not_found(
                format!("no content control '{id}'"),
                Vec::new(),
            ))
        })?;
        let part = docx
            .part(&found.part)
            .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
        let level = control_level(part.root(), &found.path, &found.element);
        if level != "block" {
            let level = if level == "inline" { "line" } else { level };
            return Err(EditError::Refused(format!(
                "{id} is a content control in a {level}, not a block: remove_control (the content-control tool's remove) takes it out"
            )));
        }
        let block = BlockAt {
            kind: BlockKind::Control,
            path: found.path.to_vec(),
        };
        return Ok((found.part, block));
    }
    let followed = doc.ids().follow(id);
    let (story, entry) = resolve(docx, &followed).map_err(EditError::Address)?;
    match entry {
        Entry::Paragraph(_) => {
            let found = find(doc, docx, id)?;
            Ok((
                found.part,
                BlockAt {
                    kind: BlockKind::Paragraph,
                    path: found.path.to_vec(),
                },
            ))
        }
        Entry::Table(k) => {
            let table = story
                .ids
                .tables()
                .get(k)
                .ok_or_else(|| EditError::Path(story.part.clone()))?;
            Ok((
                story.part.clone(),
                BlockAt {
                    kind: BlockKind::Table,
                    path: table.path.to_vec(),
                },
            ))
        }
    }
}

/// Where the control at `path` stands (docx-agent's `control_level`): `block`, `inline`,
/// `row` or `cell`.
fn control_level(root: &Arc<Element>, path: &[u32], sdt: &Element) -> &'static str {
    let mut above = path.split_last().map_or(&[][..], |(_, p)| p);
    let element_at = |p: &[u32]| if p.is_empty() { Some(root) } else { at_path(root, p) };
    let mut parent = element_at(above);
    while let Some(p) = parent.filter(|p| is_w(p, &["customXml", "sdtContent", "smartTag"])) {
        let _ = p;
        let Some((_, up)) = above.split_last() else {
            parent = None;
            break;
        };
        above = up;
        parent = element_at(above);
    }
    match parent {
        Some(p)
            if is_w(
                p,
                &["p", "hyperlink", "ins", "del", "moveTo", "moveFrom", "fldSimple", "sdt"],
            ) =>
        {
            let content = sdt.find(Some(W), "sdtContent");
            let holds = content.is_some_and(|c| c.find(Some(W), "p").is_some() || c.find(Some(W), "tbl").is_some());
            if holds {
                "block"
            } else {
                "inline"
            }
        }
        Some(p) if is_w(p, &["tbl"]) => "row",
        Some(p) if is_w(p, &["tr"]) => "cell",
        _ => "block",
    }
}

fn is_w(element: &Element, locals: &[&str]) -> bool {
    element.name().namespace() == Some(W) && locals.contains(&element.name().local())
}

/// The blocks `address` names, in document order, and their part (docx-agent's
/// `_blocks_of` for an address).
///
/// # Errors
/// The address's refusal; a span across containers or ending before it starts.
pub(crate) fn blocks_of(doc: &WordDoc, docx: &Docx, address: &str) -> Result<(String, Vec<BlockAt>)> {
    let address = expand_heading(docx, address).map_err(EditError::Address)?;
    let Some((first, last)) = span_ends(&address).map_err(EditError::Address)? else {
        let (part, block) = resolve_block(doc, docx, &address)?;
        return Ok((part, vec![block]));
    };
    let given = format!("{}..{}", first.unwrap_or(""), last.unwrap_or(""));
    let mut ends = Vec::new();
    for end in [first, last].into_iter().flatten() {
        ends.push(resolve_block(doc, docx, end)?);
    }
    let Some((part, anchor)) = ends.first().cloned() else {
        return Err(EditError::Refused(format!("{given:?}: no blocks")));
    };
    let story = docx.part(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
    let (_, above) = anchor.path.split_last().ok_or_else(|| EditError::Path(part.clone()))?;
    let container = if above.is_empty() {
        story.root()
    } else {
        at_path(story.root(), above).ok_or_else(|| EditError::Path(part.clone()))?
    };
    let siblings: Vec<BlockAt> = container
        .children()
        .iter()
        .enumerate()
        .filter_map(|(k, node)| match node {
            Node::Element(e) => kind_of(e).map(|kind| {
                let mut path = above.to_vec();
                path.push(u32::try_from(k).unwrap_or(u32::MAX));
                BlockAt { kind, path }
            }),
            _ => None,
        })
        .collect();
    let index = |b: &BlockAt| siblings.iter().position(|s| s.path == b.path);
    let (a, b) = match (first, last, ends.get(1)) {
        (Some(_), Some(_), Some((end_part, end))) => {
            let same = *end_part == part && end.path.split_last().map(|(_, p)| p) == Some(above);
            if !same {
                return Err(EditError::Refused(format!(
                    "{}: a span is blocks of one container (the body, one cell); {SPAN_SYNTAX}",
                    ooxml_base::pyfmt::repr_str(&given)
                )));
            }
            (index(&anchor), index(end))
        }
        (Some(_), None, _) => (index(&anchor), siblings.len().checked_sub(1)),
        _ => (Some(0), index(&anchor)),
    };
    let (Some(a), Some(b)) = (a, b) else {
        return Err(EditError::Path(part));
    };
    if b < a {
        return Err(EditError::Refused(format!(
            "{} ends before it starts",
            ooxml_base::pyfmt::repr_str(&given)
        )));
    }
    Ok((part, siblings.get(a..=b).unwrap_or_default().to_vec()))
}
