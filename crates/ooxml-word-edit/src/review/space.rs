//! Owns: [`Space`], the editable copies of the parts one review step changes -- each a
//! [`Work`] of a part's root, made on first use from the version the step began on --
//! with the way between a version's paths and a copy's nodes in both directions, and the
//! parts put back through the sink when the step ends.
//!
//! The review keeps its nodes across parts and cleanups (a paragraph removed in one part,
//! a note's or comment's paragraphs removed after it), so what a step removed is told by
//! which nodes are still in their tree, as docx-agent tells it by element identity.
//!
//! Must not: decide anything about a review; write but through the sink.

use crate::doc::WordDoc;
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_xml::{Element, Node, Tree};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// One part's copy: the tree it was made from and the copy.
pub(crate) struct Copy {
    tree: Tree,
    pub(crate) work: Work,
    /// The node each of the root's children was read as (none for text): a copy numbers
    /// the nodes it reads in document order, text aside.
    top: Vec<Option<Id>>,
    /// For each element of the tree a path went through (by address: the tree is held),
    /// how many nodes its children before each one read as: a place is found in one step
    /// per level, however many siblings come before it.
    before: RefCell<HashMap<usize, Arc<[usize]>>>,
}

impl Copy {
    /// How many nodes `element`'s children before its `k`th read as.
    fn before(&self, element: &Arc<Element>, k: usize) -> Option<usize> {
        let key = Arc::as_ptr(element) as usize;
        let mut memo = self.before.borrow_mut();
        let sums = memo.entry(key).or_insert_with(|| {
            let mut sum = 0;
            let mut out = Vec::with_capacity(element.children().len());
            for child in element.children().iter() {
                out.push(sum);
                sum += weight(child);
            }
            Arc::from(out)
        });
        sums.get(k).copied()
    }
}

/// How many nodes a copy reads for `node` and everything below it (text is no node).
fn weight(node: &Node) -> usize {
    match node {
        Node::Text(_) => 0,
        Node::Element(e) => 1 + e.children().iter().map(weight).sum::<usize>(),
        _ => 1,
    }
}

/// The parts a step changes.
#[derive(Default)]
pub(crate) struct Space {
    copies: BTreeMap<String, Copy>,
    /// Parts docx-agent writes whether or not they changed (marked dirty).
    dirty: Vec<String>,
}

impl Space {
    /// The copy of `part`, made from `doc` on first use (`None`: no such XML part).
    pub(crate) fn work(&mut self, doc: &WordDoc, part: &str) -> Result<Option<&mut Work>> {
        if !self.copies.contains_key(part) {
            let Some(tree) = doc.tree(part)? else {
                return Ok(None);
            };
            let work = Work::of(tree.root(), Vec::new());
            let mut top = Vec::new();
            let mut next = work.root() + 1;
            for node in tree.root().children().iter() {
                if matches!(node, Node::Text(_)) {
                    top.push(None);
                } else {
                    top.push(Some(next));
                    next += weight(node);
                }
            }
            self.copies.insert(
                part.to_owned(),
                Copy {
                    tree,
                    work,
                    top,
                    before: RefCell::new(HashMap::new()),
                },
            );
        }
        Ok(self.copies.get_mut(part).map(|c| &mut c.work))
    }

    /// The copy of `part`, if one was made.
    pub(crate) fn get(&self, part: &str) -> Option<&Work> {
        self.copies.get(part).map(|c| &c.work)
    }

    /// The copy of `part`, if one was made, to change.
    pub(crate) fn get_mut(&mut self, part: &str) -> Option<&mut Work> {
        self.copies.get_mut(part).map(|c| &mut c.work)
    }

    /// Write `part` when the step ends even if its content did not change.
    pub(crate) fn mark_dirty(&mut self, part: &str) {
        if !self.dirty.iter().any(|p| p == part) {
            self.dirty.push(part.to_owned());
        }
    }

    /// The copy's node for the element at `path` of the tree the copy was made from,
    /// whatever the copy has done since (it numbers the nodes it reads in document order).
    pub(crate) fn locate(&self, part: &str, path: &[u32]) -> Option<Id> {
        let copy = self.copies.get(part)?;
        let (&first, rest) = path.split_first()?;
        let mut node = copy.top.get(first as usize).copied().flatten()?;
        let mut green = copy.tree.root().children().get(first as usize)?;
        for &step in rest {
            let Node::Element(e) = green else { return None };
            node += 1 + copy.before(e, step as usize)?;
            green = e.children().get(step as usize)?;
        }
        let same = match green {
            Node::Element(e) => copy.work.name(node).is_some_and(|n| n == e.name()),
            _ => false,
        };
        same.then_some(node)
    }

    /// The path a node of the copy of `part` has in the tree the copy builds (green
    /// indexes: text between children counts).
    pub(crate) fn path_of(&self, part: &str, id: Id) -> Option<Vec<u32>> {
        let work = &self.copies.get(part)?.work;
        let mut path = Vec::new();
        let mut at = id;
        while let Some(parent) = work.parent(at) {
            let mut k = usize::from(!work.text(parent).is_empty());
            for child in work.children(parent) {
                if *child == at {
                    break;
                }
                k += 1 + usize::from(!work.tail(*child).is_empty());
            }
            path.push(u32::try_from(k).ok()?);
            at = parent;
        }
        (at == work.root()).then(|| {
            path.reverse();
            path
        })
    }

    /// Put every copied part that changed (or was marked dirty) back through the sink.
    ///
    /// # Errors
    /// When a part cannot be put.
    pub(crate) fn finish(&self, sink: &mut EditSink<'_, '_>) -> Result<()> {
        for (part, copy) in &self.copies {
            if !copy.work.changed() && !self.dirty.contains(part) {
                continue;
            }
            sink.keep_written(part, None, &copy.work);
            let root: Arc<Element> = copy.work.build().ok_or_else(|| EditError::Path(part.clone()))?;
            sink.put(part, copy.tree.with_root(root))?;
        }
        Ok(())
    }
}
