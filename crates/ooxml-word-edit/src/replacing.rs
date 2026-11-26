//! Owns: the blocks an edit at `replace:` replaces (a copy's, docx-agent's `copy_blocks`;
//! Markdown's, its `insert_markdown`), deleted as docx-agent deletes them (`delete_block`
//! each, by the edit's policy): first where anything follows them in their container or the
//! edit is tracked -- the new blocks then go before what followed them, so their ids never
//! meet the old ones' -- else after the new blocks are placed (a cell's last blocks: the
//! container keeps a paragraph throughout).
//!
//! Must not: delete but through [`crate::delete_write`].

use crate::names::W;
use crate::placing::{is_block, Place};
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_word_model::docx::at_path;
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// The blocks replaced, and whether they go before the new blocks are placed.
pub(crate) struct Replaced {
    /// Their ids, in order.
    pub(crate) ids: Vec<String>,
    /// Deleted before the new blocks are placed.
    pub(crate) first: bool,
    /// The ids the deletions removed.
    pub(crate) removed: Vec<String>,
}

impl Replaced {
    /// The blocks `ids` (the first at `place`), deleted now where they go first, `place`
    /// then before what followed them; `tracked`: the edit's policy tracks.
    ///
    /// # Errors
    /// A deletion's refusal; a place the part does not have.
    pub(crate) fn begin(
        sink: &mut EditSink<'_, '_>,
        place: &mut Place,
        ids: Vec<String>,
        tracked: bool,
    ) -> Result<Self> {
        let mut out = Replaced {
            ids,
            first: false,
            removed: Vec::new(),
        };
        if out.ids.is_empty() {
            return Ok(out);
        }
        // what follows the last block replaced, counted from the container's end (the
        // deletions change the children before it, never those after)
        let container = container(sink, place)?;
        let following = following(&container, place.before, out.ids.len());
        let len = container.children().len();
        out.first = tracked || following.is_some();
        if !out.first {
            return Ok(out);
        }
        let from_end = following.map(|k| len - k);
        out.delete(sink)?;
        let len = container_len(sink, place)?;
        place.before = from_end
            .map(|n| u32::try_from(len.saturating_sub(n)).map_err(|_| EditError::no_element(place.part.clone())))
            .transpose()?;
        Ok(out)
    }

    /// The blocks deleted after the new blocks are placed, where they did not go first; untracked,
    /// one block replaced by one noted (`_note_replaced`, of the new blocks' own ids
    /// `per_element`).
    ///
    /// # Errors
    /// A deletion's refusal.
    pub(crate) fn finish(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        tracked: bool,
        per_element: &[Vec<String>],
    ) -> Result<()> {
        if self.ids.is_empty() {
            return Ok(());
        }
        if !self.first {
            self.delete(sink)?;
        }
        if tracked {
            return Ok(());
        }
        let new: Vec<String> = per_element
            .iter()
            .filter_map(|ids| ids.first())
            .filter(|id| !id.is_empty())
            .cloned()
            .collect();
        sink.note_replaced(&self.ids, &new)
    }

    fn delete(&mut self, sink: &mut EditSink<'_, '_>) -> Result<()> {
        for id in &self.ids {
            let deleted = crate::delete_write::delete(sink, id)?;
            self.removed.extend(deleted.removed);
        }
        Ok(())
    }
}

/// The element `place`'s container is.
fn container(sink: &EditSink<'_, '_>, place: &Place) -> Result<Arc<Element>> {
    let tree = sink
        .doc()
        .tree(&place.part)?
        .ok_or_else(|| EditError::no_part(place.part.clone()))?;
    if place.container.is_empty() {
        return Ok(Arc::clone(tree.root()));
    }
    at_path(tree.root(), &place.container)
        .map(Arc::clone)
        .ok_or_else(|| EditError::no_element(place.part.clone()))
}

/// How many children `place`'s container has now.
fn container_len(sink: &EditSink<'_, '_>, place: &Place) -> Result<usize> {
    Ok(container(sink, place)?.children().len())
}

/// The child (index) after the `count` blocks from the child `from` on that is a block or
/// the body's section properties (`_next_block`), if any.
fn following(container: &Element, from: Option<u32>, count: usize) -> Option<usize> {
    let children = container.children();
    let element = |k: usize| match children.get(k) {
        Some(Node::Element(e)) => Some(e),
        _ => None,
    };
    let start = from? as usize;
    let last = (start..children.len())
        .filter(|k| element(*k).is_some_and(|e| is_block(e)))
        .nth(count.checked_sub(1)?)?;
    (last + 1..children.len()).find(|k| element(*k).is_some_and(|e| is_block(e) || e.name().is(Some(W), "sectPr")))
}
