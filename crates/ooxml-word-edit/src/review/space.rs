//! Owns: [`Space`], the editable copies of the parts one review step changes -- each a
//! [`Work`] of a part's root, made on first use from the version the step began on --
//! with the way between a version's paths and a copy's nodes in both directions, and the
//! parts put back through the sink when the step ends.
//!
//! A story part can instead be copied in part ([`Space::stretch`]): only the stretch of its
//! block container's children a step marks (a comment's anchor), put back as a splice of
//! those blocks, written by key -- not the whole part, which would conflict with every edit
//! made meanwhile anywhere in the story.
//!
//! The review keeps its nodes across parts and cleanups (a paragraph removed in one part,
//! a note's or comment's paragraphs removed after it), so what a step removed is told by
//! which nodes are still in their tree, as docx-agent tells it by element identity.
//!
//! Must not: decide anything about a review; write but through the sink.

use crate::doc::WordDoc;
use crate::sink::EditSink;
use ooxml_word_model::docx::at_path;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_xml::{Element, Node, Tree};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// The stretch of a part's children a copy holds: the place of their parent, and the
/// first and last child's index there.
#[derive(Debug, Clone)]
struct Within {
    parent: Vec<u32>,
    from: u32,
    to: u32,
}

/// One part's copy: the tree it was made from and the copy.
pub(crate) struct Copy {
    tree: Tree,
    pub(crate) work: Work,
    /// The stretch the copy holds (its root a copy of their parent with only them), or
    /// `None` for the whole part.
    within: Option<Within>,
    /// The node each child the copy holds of its root (the stretch's parent) was read as
    /// (none for text): a copy numbers the nodes it reads in document order, text aside.
    top: Vec<Option<Id>>,
    /// For each element of the tree a path went through (by address: the tree is held),
    /// how many nodes its children before each one read as: a place is found in one step
    /// per level, however many siblings come before it.
    before: RefCell<HashMap<usize, Arc<[usize]>>>,
}

impl Copy {
    fn new(tree: Tree, holder: &Arc<Element>, within: Option<Within>) -> Self {
        let work = Work::of(holder, Vec::new());
        let mut top = Vec::new();
        let mut next = work.root() + 1;
        for node in holder.children().iter() {
            if matches!(node, Node::Text(_)) {
                top.push(None);
            } else {
                top.push(Some(next));
                next += weight(node);
            }
        }
        Copy {
            tree,
            work,
            within,
            top,
            before: RefCell::new(HashMap::new()),
        }
    }

    /// The element of the tree whose children the copy's root holds.
    fn holder(&self) -> Option<&Arc<Element>> {
        match &self.within {
            Some(within) if !within.parent.is_empty() => at_path(self.tree.root(), &within.parent),
            _ => Some(self.tree.root()),
        }
    }

    /// The element whose children the copy's root holds, the index of the first one it
    /// holds, and `path` below that element (`None`: outside the copy).
    fn holding<'p>(&self, path: &'p [u32]) -> Option<(&Arc<Element>, u32, &'p [u32])> {
        match &self.within {
            None => Some((self.tree.root(), 0, path)),
            Some(within) => {
                let rest = path.strip_prefix(within.parent.as_slice())?;
                let first = *rest.first()?;
                if first < within.from || first > within.to {
                    return None;
                }
                Some((self.holder()?, within.from, rest))
            }
        }
    }

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
            let root = Arc::clone(tree.root());
            self.copies.insert(part.to_owned(), Copy::new(tree, &root, None));
        }
        Ok(self.copies.get_mut(part).map(|c| &mut c.work))
    }

    /// Copy only the stretch of `part`'s block container (`container`, the body's place or
    /// the root's) from the block holding the first of `paths` to the block holding the
    /// last (`through`: on to that container child at least); put back by
    /// [`Space::finish`] as a splice of those blocks, written by their keys. `false` when
    /// a copy of the part was made already that does not hold the stretch, or a path is
    /// not below the container.
    ///
    /// # Errors
    /// When the part is not XML.
    pub(crate) fn stretch(
        &mut self,
        doc: &WordDoc,
        part: &str,
        container: &[u32],
        paths: &[&[u32]],
        through: Option<u32>,
    ) -> Result<bool> {
        let tops: Option<Vec<u32>> = paths
            .iter()
            .map(|p| p.strip_prefix(container).and_then(|rest| rest.first().copied()))
            .collect();
        let Some(tops) = tops else { return Ok(false) };
        let (Some(&from), Some(&last)) = (tops.iter().min(), tops.iter().max()) else {
            return Ok(false);
        };
        let to = through.map_or(last, |t| t.max(last));
        if let Some(copy) = self.copies.get(part) {
            let mut through_place = container.to_vec();
            through_place.push(to);
            return Ok(paths.iter().all(|p| copy.holding(p).is_some()) && copy.holding(&through_place).is_some());
        }
        let Some(tree) = doc.tree(part)? else {
            return Ok(false);
        };
        let parent = if container.is_empty() {
            Arc::clone(tree.root())
        } else {
            match at_path(tree.root(), container) {
                Some(parent) => Arc::clone(parent),
                None => return Ok(false),
            }
        };
        let count = parent.children().len();
        let to = to.min(u32::try_from(count.saturating_sub(1)).unwrap_or(u32::MAX));
        if from > to {
            return Ok(false);
        }
        let slice: Vec<Node> = parent
            .children()
            .iter()
            .skip(from as usize)
            .take((to - from) as usize + 1)
            .cloned()
            .collect();
        let holder = Arc::new(Element::from_head(
            Arc::clone(parent.head()),
            ooxml_xml::Children::from_vec(slice),
        ));
        let within = Within {
            parent: container.to_vec(),
            from,
            to,
        };
        self.copies.insert(part.to_owned(), Copy::new(tree, &holder, Some(within)));
        Ok(true)
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
        let (holder, offset, path) = copy.holding(path)?;
        let (&first, rest) = path.split_first()?;
        let mut node = copy.top.get(first.checked_sub(offset)? as usize).copied().flatten()?;
        let mut green = holder.children().get(first as usize)?;
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
        let copy = self.copies.get(part)?;
        let work = &copy.work;
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
        if at != work.root() {
            return None;
        }
        path.reverse();
        if let Some(within) = &copy.within {
            // the copy's root holds the stretch: its first child is the parent's `from`th
            let first = path.first_mut()?;
            *first += within.from;
            let mut full = within.parent.clone();
            full.append(&mut path);
            return Some(full);
        }
        Some(path)
    }

    /// Put every copied part that changed (or was marked dirty) back through the sink.
    ///
    /// # Errors
    /// When a part cannot be put.
    pub(crate) fn finish(&self, sink: &mut EditSink<'_, '_>) -> Result<()> {
        self.put_back(sink, false)
    }

    /// Put back only the parts whose XML changed: a copy edited back to what it was, or
    /// marked dirty, is left as the version has it.
    ///
    /// # Errors
    /// When a part cannot be put.
    pub(crate) fn finish_changed(&self, sink: &mut EditSink<'_, '_>) -> Result<()> {
        self.put_back(sink, true)
    }

    /// Put `part`'s copy back now (if it changed or was marked dirty) and forget it: what
    /// writes the part through the sink next sees the copy's changes, and a later use
    /// copies the part again.
    ///
    /// # Errors
    /// When the part cannot be put.
    pub(crate) fn put_part(&mut self, sink: &mut EditSink<'_, '_>, part: &str) -> Result<()> {
        let Some(copy) = self.copies.remove(part) else {
            return Ok(());
        };
        if copy.work.changed() || self.dirty.iter().any(|p| p == part) {
            copy.put(sink, part, false)?;
        }
        Ok(())
    }

    fn put_back(&self, sink: &mut EditSink<'_, '_>, changed_only: bool) -> Result<()> {
        for (part, copy) in &self.copies {
            if !copy.work.changed() && !self.dirty.contains(part) {
                continue;
            }
            copy.put(sink, part, changed_only)?;
        }
        Ok(())
    }
}

impl Copy {
    /// Put the copy back through `sink`: the whole part, or the stretch it holds as a
    /// splice of those children writing their blocks' keys (`changed_only`: not when the
    /// XML is as it was).
    fn put(&self, sink: &mut EditSink<'_, '_>, part: &str, changed_only: bool) -> Result<()> {
        sink.keep_written(part, None, &self.work);
        let root: Arc<Element> = self.work.build().ok_or_else(|| EditError::Path(part.to_owned()))?;
        let Some(within) = &self.within else {
            if changed_only && *root == **self.tree.root() {
                return Ok(());
            }
            return sink.put(part, self.tree.with_root(root));
        };
        let holder = self.holder().ok_or_else(|| EditError::Path(part.to_owned()))?;
        let old = holder
            .children()
            .iter()
            .skip(within.from as usize)
            .take((within.to - within.from) as usize + 1);
        if changed_only && old.eq(root.children().iter()) {
            return Ok(());
        }
        let docx = sink.docx()?;
        let keys = crate::delete::stretch_keys(&docx, part, &within.parent, (within.from, within.to));
        let count = (within.to - within.from) as usize + 1;
        let nodes = root.children().to_vec();
        sink.splice_children(
            part,
            (&within.parent, within.from as usize, count),
            nodes,
            keys,
            Vec::new(),
        )
    }
}
