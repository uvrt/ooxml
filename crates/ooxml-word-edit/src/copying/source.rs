//! Owns: the source blocks a copy names, as docx-agent's importer selects them
//! (`_source_blocks`): one id, `first..last` (blocks of one container), or a list of
//! either, all in one story; and, per block, the ids of what it holds that the reply maps
//! to the copy's (`_block_ids`: the block, then every paragraph in it).
//!
//! Must not: change anything.

use super::CopyRange;
use crate::blocks::{resolve_block_with, BlockAt};
use crate::names::W;
use crate::placing::is_block;
use crate::{EditError, Ids, Result};
use ooxml_base::pyfmt::repr_str;
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// The blocks a copy takes, in order, with their story part.
pub(crate) struct Selected {
    /// The story part.
    pub(crate) part: String,
    /// Each block's path in the part and its element.
    pub(crate) blocks: Vec<(Vec<u32>, Arc<Element>)>,
}

/// The blocks `range` names in the document read as `docx`, whose renames are `ids`'.
///
/// # Errors
/// The address's refusal of an id; blocks in two stories; a range across containers.
pub(crate) fn select(ids: &Ids, docx: &Docx, range: &CopyRange) -> Result<Selected> {
    let items: Vec<&str> = match range {
        CopyRange::Blocks(one) => vec![one.as_str()],
        CopyRange::Ids(list) => list.iter().map(String::as_str).collect(),
    };
    if items.is_empty() {
        return Err(EditError::Refused("copy_blocks needs at least one block id".to_owned()));
    }
    let mut part: Option<String> = None;
    let mut paths: Vec<Vec<u32>> = Vec::new();
    for item in items {
        let (first, last) = match item.split_once("..") {
            Some((first, last)) => (first, Some(last)),
            None => (item, None),
        };
        let (where_, entry) = resolve_block_with(ids, docx, first)?;
        match &part {
            None => part = Some(where_.clone()),
            Some(p) if *p != where_ => {
                return Err(EditError::Refused("the blocks copied are in one story".to_owned()));
            }
            Some(_) => {}
        }
        let chosen = match last {
            None => vec![entry.path],
            Some(last) => span(ids, docx, item, (&where_, &entry), last)?,
        };
        for path in chosen {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    let part = part.unwrap_or_default();
    let story = docx.part(&part).ok_or_else(|| EditError::no_part(part.clone()))?;
    let blocks = paths
        .into_iter()
        .map(|path| {
            let element = at_path(story.root(), &path).ok_or_else(|| EditError::no_element(part.clone()))?;
            Ok((path, Arc::clone(element)))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Selected { part, blocks })
}

/// The blocks from `first` to the block `last` names, of one container, in order.
fn span(ids: &Ids, docx: &Docx, item: &str, (part, first): (&str, &BlockAt), last: &str) -> Result<Vec<Vec<u32>>> {
    let (other_part, other) = resolve_block_with(ids, docx, last)?;
    let parent = |b: &BlockAt| b.path.split_last().map(|(_, p)| p.to_vec());
    if other_part != part || parent(&other) != parent(first) {
        return Err(EditError::Refused(format!(
            "{}: a range copied is blocks of one container",
            repr_str(item)
        )));
    }
    let above = parent(first).unwrap_or_default();
    let story = docx.part(part).ok_or_else(|| EditError::no_part(part.to_owned()))?;
    let container = if above.is_empty() {
        story.root()
    } else {
        at_path(story.root(), &above).ok_or_else(|| EditError::no_element(part.to_owned()))?
    };
    let siblings: Vec<Vec<u32>> = container
        .children()
        .iter()
        .enumerate()
        .filter(|(_, n)| matches!(n, Node::Element(e) if is_block(e)))
        .filter_map(|(k, _)| {
            let mut path = above.clone();
            path.push(u32::try_from(k).ok()?);
            Some(path)
        })
        .collect();
    let index = |b: &BlockAt| siblings.iter().position(|s| *s == b.path);
    let (Some(mut a), Some(mut b)) = (index(first), index(&other)) else {
        return Err(EditError::no_element(part.to_owned()));
    };
    if b < a {
        std::mem::swap(&mut a, &mut b);
    }
    Ok(siblings.get(a..=b).unwrap_or_default().to_vec())
}

/// Per block, the ids of what it holds that a copy maps (`_block_ids`): the block's own
/// (a paragraph's or table's; `""` for a control or custom XML), then every paragraph in
/// it, in document order (`""` for one the part's index does not hold).
pub(crate) fn block_ids(docx: &Docx, selected: &Selected) -> Vec<Vec<String>> {
    let Some(story) = docx.part(&selected.part) else {
        return Vec::new();
    };
    let paragraph = |path: &[u32]| story.ids.paragraph_at(path).map(|p| p.id).unwrap_or_default();
    let table = |path: &[u32]| {
        story
            .ids
            .tables()
            .iter()
            .find(|t| *t.path == *path)
            .map(|t| t.id.clone())
            .unwrap_or_default()
    };
    selected
        .blocks
        .iter()
        .map(|(path, element)| {
            let own = if element.name().is(Some(W), "p") {
                paragraph(path)
            } else if element.name().is(Some(W), "tbl") {
                table(path)
            } else {
                String::new()
            };
            let mut ids = vec![own];
            let mut inner = Vec::new();
            paragraph_paths(element, path, &mut inner);
            ids.extend(inner.iter().map(|p| paragraph(p)));
            ids
        })
        .collect()
}

/// The paths of every `w:p` below `element` (at `path`), in document order.
fn paragraph_paths(element: &Element, path: &[u32], out: &mut Vec<Vec<u32>>) {
    for (k, child) in element.children().iter().enumerate() {
        let Node::Element(e) = child else { continue };
        let Ok(k) = u32::try_from(k) else { continue };
        let mut at = path.to_vec();
        at.push(k);
        if e.name().is(Some(W), "p") {
            out.push(at.clone());
        }
        paragraph_paths(e, &at, out);
    }
}
