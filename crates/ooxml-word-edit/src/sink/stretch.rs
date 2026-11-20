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
use ooxml_xml::Node;
use std::collections::HashMap;
use std::sync::Arc;

/// A run of children that differs: old `[from, to)` replaced by new `[into, until)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hunk {
    from: usize,
    to: usize,
    into: usize,
    until: usize,
}

/// Whether two children are the same node: an element by identity (the version's own,
/// untouched), text by its value.
fn same(a: &Node, b: &Node) -> bool {
    match (a, b) {
        (Node::Element(x), Node::Element(y)) => Arc::ptr_eq(x, y),
        (Node::Text(x), Node::Text(y)) => x == y,
        _ => false,
    }
}

/// The runs of `old` that `new` replaces, in order: equal children are kept, and past a
/// difference the nearest element both hold again (fewest children changed) ends the run.
fn hunks(old: &[Node], new: &[Node]) -> Vec<Hunk> {
    let place: HashMap<*const ooxml_xml::Element, usize> = new
        .iter()
        .enumerate()
        .filter_map(|(k, n)| match n {
            Node::Element(e) => Some((Arc::as_ptr(e), k)),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        if old.get(i).zip(new.get(j)).is_some_and(|(a, b)| same(a, b)) {
            i += 1;
            j += 1;
            continue;
        }
        // the sync point (k in old, m in new) changing the fewest children
        let mut best: Option<(usize, usize)> = None;
        for (k, node) in old.iter().enumerate().skip(i) {
            if best.is_some_and(|(bk, bm)| k - i >= (bk - i) + (bm - j)) {
                break;
            }
            let Node::Element(e) = node else { continue };
            if let Some(&m) = place.get(&Arc::as_ptr(e)) {
                let better = best.is_none_or(|(bk, bm)| (k - i) + (m - j) < (bk - i) + (bm - j));
                if m >= j && better {
                    best = Some((k, m));
                }
            }
        }
        let (k, m) = best.unwrap_or((old.len(), new.len()));
        out.push(Hunk {
            from: i,
            to: k,
            into: j,
            until: m,
        });
        (i, j) = (k, m);
    }
    if i < old.len() || j < new.len() {
        out.push(Hunk {
            from: i,
            to: old.len(),
            into: j,
            until: new.len(),
        });
    }
    out
}

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
        let runs = hunks(&old, &nodes);
        let placed = |d: &Draw| runs.iter().any(|h| (h.into..h.until).contains(&(d.node as usize)));
        if !draws.iter().all(placed) {
            // a draw into a child kept as it is: write the stretch whole
            let keys = keys_of(docx, part, parent, (start, start + count), extra);
            return self.splice_children(part, (parent, start, count), nodes, keys, draws);
        }
        // from the last run back: the places of the runs before it stay as they were
        for hunk in runs.iter().rev() {
            let inserted: Vec<Node> = nodes.get(hunk.into..hunk.until).unwrap_or_default().to_vec();
            let mine: Vec<Draw> = draws
                .iter()
                .filter(|d| (hunk.into..hunk.until).contains(&(d.node as usize)))
                .map(|d| Draw {
                    node: d.node - u32::try_from(hunk.into).unwrap_or(0),
                    ..d.clone()
                })
                .collect();
            let (from, to) = (start + hunk.from, start + hunk.to);
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

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use ooxml_xml::{Atom, Element, QName};

    fn el(name: &str) -> Node {
        Node::Element(Arc::new(Element::new(
            Arc::new(QName::new(None, name, None)),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )))
    }

    #[test]
    fn a_block_moved_from_the_start_to_the_end_is_two_runs() {
        let old: Vec<Node> = (0..6).map(|k| el(&format!("b{k}"))).collect();
        let mut new: Vec<Node> = old[1..].to_vec();
        new.push(old[0].clone());
        let found = hunks(&old, &new);
        assert_eq!(
            found,
            vec![
                Hunk {
                    from: 0,
                    to: 1,
                    into: 0,
                    until: 0
                },
                Hunk {
                    from: 6,
                    to: 6,
                    into: 5,
                    until: 6
                }
            ]
        );
    }

    #[test]
    fn a_changed_child_and_one_inserted_are_runs_of_their_own() {
        let old: Vec<Node> = (0..5).map(|k| el(&format!("b{k}"))).collect();
        let new = vec![
            old[0].clone(),
            el("new"),
            old[1].clone(),
            old[2].clone(),
            old[3].clone(),
            el("b4 changed"),
            Node::Text(Atom::from("\n")),
        ];
        let found = hunks(&old, &new);
        assert_eq!(
            found,
            vec![
                Hunk {
                    from: 1,
                    to: 1,
                    into: 1,
                    until: 2
                },
                Hunk {
                    from: 4,
                    to: 5,
                    into: 5,
                    until: 7
                }
            ]
        );
    }
}
