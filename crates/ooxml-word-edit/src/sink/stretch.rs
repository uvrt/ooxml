//! Owns: a stretch of a container's children written back as the few splices that change
//! it ([`EditSink::splice_stretch`]): an edit that works on a copy of the stretch between
//! two places (a block moved to the end of the body, a section break whose split section
//! is the body's last) gives back every child of it, nearly all of them the version's own
//! nodes; only the runs of children that differ are spliced, each with the keys of the
//! blocks it replaces, so the commit's indexes, tally and write set follow the change, not
//! the stretch.
//!
//! Must not: change what the stretch becomes (the children written are the ones given, in
//! order), or write but through the sink.

use super::EditSink;
use crate::doc::{Draw, Key};
use crate::set_text::block_key;
use crate::{EditError, Result};
use ooxml_word_model::docx::Docx;
use ooxml_xml::{changed_runs, Children, Node};

impl EditSink<'_, '_> {
    /// Replace the children `[start, start + count)` of the element at `parent` of `part`
    /// by `nodes` -- as [`EditSink::splice_children`] would -- writing only the runs that
    /// differ, each with the block keys of what it replaces (or, adding only, of its
    /// neighbours) and `extra` keys; `draws` (on `nodes`) go with the run holding their node.
    ///
    /// # Errors
    /// When the part has no such element or children.
    pub(crate) fn splice_stretch(
        &mut self,
        docx: &Docx,
        part: &str,
        (parent, start, count): (&[u32], usize, usize),
        nodes: Vec<Node>,
        extra: &[Key],
        draws: Vec<Draw>,
    ) -> Result<()> {
        let tree = self
            .tx
            .doc()
            .tree(part)?
            .ok_or_else(|| EditError::NoPart(part.to_owned()))?;
        let element =
            ooxml_word_model::docx::at_path(tree.root(), parent).ok_or_else(|| EditError::Path(part.to_owned()))?;
        let old: Vec<Node> = element.children().iter().skip(start).take(count).cloned().collect();
        if old.len() != count {
            return Err(EditError::Path(part.to_owned()));
        }
        let runs = changed_runs(&Children::from_vec(old), &Children::from_vec(nodes.clone()));
        let placed = |d: &Draw| runs.iter().any(|r| r.new.contains(&(d.node as usize)));
        if !draws.iter().all(placed) {
            // a draw into a child kept as it is: write the stretch whole
            let keys = keys_of(docx, part, parent, (start, start + count), extra);
            return self.splice_children(part, (parent, start, count), nodes, keys, draws);
        }
        // from the last run back: the places of the runs before it stay as they were
        for run in runs.iter().rev() {
            let inserted: Vec<Node> = nodes.get(run.new.clone()).unwrap_or_default().to_vec();
            let mine: Vec<Draw> = draws
                .iter()
                .filter(|d| run.new.contains(&(d.node as usize)))
                .map(|d| Draw {
                    node: d.node - u32::try_from(run.new.start).unwrap_or(0),
                    ..d.clone()
                })
                .collect();
            let (from, to) = (start + run.old.start, start + run.old.end);
            let around = if from == to {
                (from.saturating_sub(1), (to + 1).min(start + count))
            } else {
                (from, to)
            };
            let keys = keys_of(docx, part, parent, around, extra);
            self.splice_children(part, (parent, from, to - from), inserted, keys, mine)?;
        }
        Ok(())
    }
}

/// The block keys of the children `[from, to)` of `parent`, and `extra`.
fn keys_of(docx: &Docx, part: &str, parent: &[u32], (from, to): (usize, usize), extra: &[Key]) -> Vec<Key> {
    let mut keys = Vec::new();
    if let (Some(story), Ok(first), Ok(last)) = (
        docx.part(part),
        u32::try_from(from),
        u32::try_from(to.saturating_sub(1)),
    ) {
        if to > from {
            let (paragraphs, tables) = story.ids.within_stretch(parent, (first, last));
            keys.extend(paragraphs.iter().map(|p| block_key(part, &p.id)));
            keys.extend(tables.iter().map(|t| block_key(part, &t.id)));
        }
    }
    keys.extend(extra.iter().cloned());
    keys
}
