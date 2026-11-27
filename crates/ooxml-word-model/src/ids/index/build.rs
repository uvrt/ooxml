//! Owns: making a part's block index -- from scratch ([`PartIds::new`]) or from an earlier
//! version's ([`PartIds::after`]), where only the container children the new version does
//! not share are walked again and the index's persistent structures (the segment tree, the
//! paraId occurrences) are changed in place of being rebuilt -- with the order keys
//! (labels) segments are found by.
//!
//! Must not: name blocks (the index does, on demand) or read outside the part's tree.

use super::segment::Body;
use super::{Count, PartIds, Seg, LABEL_AFTER, LABEL_BEFORE};
use crate::ids::W;
use ooxml_derive::{MonoidTree, PersistentMap};
use ooxml_xml::{Children, Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// The spacing of labels given afresh: room for 2^20 insertions at one place.
const GAP: u64 = 1 << 20;

/// The block container below `root`: its place (the body's, for a main part) and the
/// element, `None` for the root itself.
pub(super) fn container(root: &Element) -> Option<(usize, &Arc<Element>)> {
    root.children()
        .iter()
        .enumerate()
        .find_map(|(k, n)| n.as_element().filter(|e| e.name().is(Some(W), "body")).map(|e| (k, e)))
}

/// The container's children.
pub(super) fn container_children(root: &Element, at: Option<usize>) -> Option<&Children> {
    match at {
        None => Some(root.children()),
        Some(k) => root.children().get(k)?.as_element().map(|e| e.children()),
    }
}

impl PartIds {
    /// The index of the part whose root is `root`, for the story `story`.
    #[must_use]
    pub fn new(story: &str, root: &Element) -> PartIds {
        let at = container(root).map(|(k, _)| k);
        let (before, after) = match at {
            Some(k) => Body::outer(root, Some(k)),
            None => (Body::default(), Body::default()),
        };
        let segments: MonoidTree<Seg> = container_children(root, at)
            .map(|children| {
                children
                    .iter()
                    .zip(1u64..)
                    .map(|(node, k)| Seg {
                        label: k * GAP,
                        body: Arc::new(Body::of(node)),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut out = PartIds {
            story: Arc::from(story),
            container: at
                .and_then(|k| u32::try_from(k).ok())
                .map_or_else(Vec::new, |k| vec![k])
                .into(),
            before: Arc::new(before),
            after: Arc::new(after),
            segments,
            occurrences: PersistentMap::new(),
            repeated: 0,
        };
        out.count_all();
        out
    }

    /// The index of `new_root`, a later version of the part `self` indexes (whose root was
    /// `old_root`): equal to [`PartIds::new`]'s, every container child the new version
    /// shares with the old one (the same allocation) keeping its segment -- an edit that
    /// path-copies one block walks that block, and changes the index in O(log n).
    #[must_use]
    pub fn after(&self, old_root: &Element, new_root: &Element) -> PartIds {
        let (old_at, new_at) = (container(old_root).map(|(k, _)| k), container(new_root).map(|(k, _)| k));
        let same_place = old_at == new_at
            && self.container.len() == usize::from(new_at.is_some())
            && self.segments.len() == container_children(old_root, old_at).map_or(0, Children::len);
        let (Some(old), Some(new)) = (
            container_children(old_root, old_at),
            container_children(new_root, new_at),
        ) else {
            return PartIds::new(&self.story, new_root);
        };
        if !same_place {
            return PartIds::new(&self.story, new_root);
        }
        let mut out = self.clone();
        if let Some(k) = new_at {
            let (before, after) = Body::outer(new_root, Some(k));
            out.before = Arc::new(before);
            out.after = Arc::new(after);
        }
        // the last run first: the places before it are still the old version's
        for run in ooxml_xml::changed_runs(old, new).into_iter().rev() {
            let fresh: Vec<Node> = run.new.filter_map(|k| new.get(k).cloned()).collect();
            out.replace(run.old.start, run.old.end, &fresh);
        }
        out
    }

    /// Every paraId occurrence counted again, from the segments (a new index, or after a
    /// relabelling).
    fn count_all(&mut self) {
        let mut lists: HashMap<Arc<str>, Vec<u64>> = HashMap::new();
        let mut add = |body: &Body, label: u64| {
            for upper in body.blocks.iter().filter_map(|b| b.upper.as_ref()) {
                lists.entry(Arc::clone(upper)).or_default().push(label);
            }
        };
        add(&self.before, LABEL_BEFORE);
        for seg in self.segments.iter() {
            add(&seg.body, seg.label);
        }
        add(&self.after, LABEL_AFTER);
        let mut occurrences = PersistentMap::new();
        let mut repeated = 0;
        for (upper, labels) in lists {
            repeated += usize::from(labels.len() > 1);
            occurrences.insert(upper, Arc::<[u64]>::from(labels));
        }
        self.occurrences = occurrences;
        self.repeated = repeated;
    }

    /// Container children `start..old_end` replaced by `fresh`.
    fn replace(&mut self, start: usize, old_end: usize, fresh: &[Node]) {
        let removed: Vec<Seg> = (start..old_end).filter_map(|k| self.segments.get(k).cloned()).collect();
        // a child that is an old one of the stretch keeps its walk
        let reusable: HashMap<usize, &Arc<Body>> = removed
            .iter()
            .filter_map(|s| s.body.node.as_ref().map(|n| (node_key(n), &s.body)))
            .collect();
        let bodies: Vec<Arc<Body>> = fresh
            .iter()
            .map(|node| match reusable.get(&node_key(node)).filter(|b| b.is_of(node)) {
                Some(body) => Arc::clone(body),
                None => Arc::new(Body::of(node)),
            })
            .collect();
        let low = start
            .checked_sub(1)
            .and_then(|k| self.segments.get(k))
            .map_or(0, |s| s.label);
        let high = self.segments.get(old_end).map_or(u64::MAX - GAP, |s| s.label);
        let Some(labels) = labels_between(&removed, low, high, bodies.len()) else {
            self.splice(start, old_end, &removed, bodies, None);
            self.relabel();
            return;
        };
        self.splice(start, old_end, &removed, bodies, Some(labels));
    }

    /// The segment tree with `removed` (at `start..old_end`) replaced by `bodies` labelled
    /// `labels` (`None`: labels to be given by a relabelling), the occurrences kept.
    fn splice(
        &mut self,
        start: usize,
        old_end: usize,
        removed: &[Seg],
        bodies: Vec<Arc<Body>>,
        labels: Option<Vec<u64>>,
    ) {
        for seg in removed {
            self.forget(seg);
        }
        let new: Vec<Seg> = bodies
            .into_iter()
            .enumerate()
            .map(|(k, body)| Seg {
                label: labels.as_ref().and_then(|l| l.get(k).copied()).unwrap_or(0),
                body,
            })
            .collect();
        let common = (old_end - start).min(new.len());
        for (k, seg) in new.iter().take(common).enumerate() {
            // set in place: the same number of children, the tree keeps its shape
            let _ = self.segments.set(start + k, seg.clone());
        }
        for _ in start + common..old_end {
            let _ = self.segments.remove(start + common);
        }
        for (k, seg) in new.iter().enumerate().skip(common) {
            let _ = self.segments.insert(start + k, seg.clone());
        }
        if labels.is_some() {
            for seg in &new {
                self.remember(seg);
            }
        }
    }

    /// Every segment labelled afresh, in order, and the occurrences counted again.
    fn relabel(&mut self) {
        let segments: MonoidTree<Seg> = self
            .segments
            .iter()
            .zip(1u64..)
            .map(|(seg, k)| Seg {
                label: k * GAP,
                body: Arc::clone(&seg.body),
            })
            .collect();
        self.segments = segments;
        self.count_all();
    }

    /// Take a segment's paraIds out of the occurrences.
    fn forget(&mut self, seg: &Seg) {
        for upper in seg.body.blocks.iter().filter_map(|b| b.upper.as_ref()) {
            let Some(list) = self.occurrences.get(upper).cloned() else {
                continue;
            };
            let mut list = list.to_vec();
            if let Some(at) = list.iter().position(|l| *l == seg.label) {
                list.remove(at);
            }
            if list.len() == 1 {
                self.repeated = self.repeated.saturating_sub(1);
            }
            if list.is_empty() {
                self.occurrences.remove(upper);
            } else {
                self.occurrences.insert(Arc::clone(upper), list.into());
            }
        }
    }

    /// Put a segment's paraIds in the occurrences, in label order.
    fn remember(&mut self, seg: &Seg) {
        for upper in seg.body.blocks.iter().filter_map(|b| b.upper.as_ref()) {
            let mut list = self.occurrences.get(upper).map(|l| l.to_vec()).unwrap_or_default();
            let at = list.partition_point(|l| *l <= seg.label);
            list.insert(at, seg.label);
            if list.len() == 2 {
                self.repeated += 1;
            }
            self.occurrences.insert(Arc::clone(upper), list.into());
        }
    }
}

/// A node's allocation, as a number.
fn node_key(node: &Node) -> usize {
    match node {
        Node::Element(e) => Arc::as_ptr(e).cast::<u8>() as usize,
        Node::Text(t) | Node::Comment(t) | Node::EntityRef(t) => Arc::as_ptr(t).cast::<u8>() as usize,
        Node::Pi(p) => Arc::as_ptr(p).cast::<u8>() as usize,
    }
}

/// `count` labels in `(low, high)`, in order: the removed segments' own first (they are
/// there already), then spread over the room after the last of them; `None` when there is
/// no room.
fn labels_between(removed: &[Seg], low: u64, high: u64, count: usize) -> Option<Vec<u64>> {
    let mut out: Vec<u64> = removed.iter().take(count).map(|s| s.label).collect();
    let rest = count - out.len();
    if rest == 0 {
        return Some(out);
    }
    let from = out.last().copied().unwrap_or(low);
    let room = high.checked_sub(from)?;
    let step = room / (rest as u64 + 1);
    if step == 0 {
        return None;
    }
    out.extend((1..=rest as u64).map(|k| from + k * step));
    Some(out)
}

impl Count {
    /// A segment's contribution.
    pub(super) fn of(seg: &Seg) -> Count {
        Count {
            paragraphs: seg.body.paragraphs.len(),
            tables: seg.body.tables.len(),
            unstamped: seg.body.unstamped as usize,
            names: std::array::from_fn(|k| u32::from(seg.body.top_name() == Some(k))),
            last: seg.label,
        }
    }
}
