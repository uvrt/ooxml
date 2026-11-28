//! Owns: what the changes feed knows of a document's versions without keeping any history
//! (design section 2, "No history"): per part, per block and per page group the seq it last
//! changed at, a removed block or group kept as a tombstone with the seq it went at, and
//! the floor below which a `since` can no longer be answered exactly ([`DocRecords`],
//! [`PageRecords`]); how a version's pages as the feed writes them advance the records
//! ([`PageRecords::advanced`]: by each group's digest, so a group only moved keeps its
//! content's seq), and what changed after a `since` ([`PageRecords::since`],
//! [`DocRecords::since`]).
//!
//! Must not: lay out or paint, hold a group's SVG (the cache does), or answer below its
//! floor.

use ooxml_base::{Digest, DigestKey};
use ooxml_word_layout::paint::feed::{FeedFrame, FeedGroup, FeedPage};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// What a group's content and resources are digested under.
const KEY: DigestKey = DigestKey::new(0x6665_6564_6772_6f75, 0x7073_7667_6469_6765);

/// The digest of a group's content and resources.
pub(crate) fn digest(group: &FeedGroup) -> Digest {
    let mut bytes = Vec::with_capacity(group.content.len() + group.defs.len() + 1);
    bytes.extend_from_slice(group.content.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(group.defs.as_bytes());
    Digest::of(KEY, &bytes)
}

/// One group of a page as last written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GroupRecord {
    /// Its id.
    pub(crate) id: String,
    /// Its offset.
    pub(crate) dy: i64,
    /// Its content's and resources' digest.
    pub(crate) digest: Digest,
    /// The seq its content or resources last changed at (it came then, at the latest).
    pub(crate) changed: u64,
    /// The seq its offset last changed at.
    pub(crate) moved: u64,
}

/// One page as last written: its frame, its groups in order, the groups it lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PageRecord {
    /// Its frame.
    pub(crate) frame: FeedFrame,
    /// Its groups, in drawing order.
    pub(crate) groups: Vec<GroupRecord>,
    /// The seq its groups' order last changed at.
    pub(crate) order: u64,
    /// The groups it lost, each with the seq it went at (none it holds again).
    pub(crate) removed: Vec<(String, u64)>,
    /// The seq anything on it last changed at.
    pub(crate) changed: u64,
}

impl PageRecord {
    /// A page first written at `seq`.
    fn new(page: &FeedPage, seq: u64) -> PageRecord {
        PageRecord {
            frame: page.frame.clone(),
            groups: page
                .groups
                .iter()
                .map(|g| GroupRecord {
                    id: g.id.clone(),
                    dy: g.dy,
                    digest: digest(g),
                    changed: seq,
                    moved: seq,
                })
                .collect(),
            order: seq,
            removed: Vec::new(),
            changed: seq,
        }
    }

    /// This page written again at `seq` as `page`: each group kept by id, its content's seq
    /// moved on where its digest differs, its offset's where that does; a group gone kept as
    /// a tombstone. Itself where nothing differs.
    fn again(self: &Arc<PageRecord>, page: &FeedPage, seq: u64) -> Arc<PageRecord> {
        let before: HashMap<&str, &GroupRecord> = self.groups.iter().map(|g| (g.id.as_str(), g)).collect();
        let groups: Vec<GroupRecord> = page
            .groups
            .iter()
            .map(|g| {
                let digest = digest(g);
                match before.get(g.id.as_str()) {
                    Some(was) => GroupRecord {
                        id: g.id.clone(),
                        dy: g.dy,
                        digest,
                        changed: if was.digest == digest { was.changed } else { seq },
                        moved: if was.dy == g.dy { was.moved } else { seq },
                    },
                    None => GroupRecord {
                        id: g.id.clone(),
                        dy: g.dy,
                        digest,
                        changed: seq,
                        moved: seq,
                    },
                }
            })
            .collect();
        let now: std::collections::HashSet<&str> = groups.iter().map(|g| g.id.as_str()).collect();
        let mut removed: Vec<(String, u64)> = self
            .removed
            .iter()
            .filter(|(id, _)| !now.contains(id.as_str()))
            .cloned()
            .collect();
        removed.extend(
            self.groups
                .iter()
                .filter(|g| !now.contains(g.id.as_str()))
                .map(|g| (g.id.clone(), seq)),
        );
        let same_order =
            groups.len() == self.groups.len() && groups.iter().zip(&self.groups).all(|(a, b)| a.id == b.id);
        let made = PageRecord {
            frame: page.frame.clone(),
            order: if same_order { self.order } else { seq },
            changed: seq,
            groups,
            removed,
        };
        let same = made.frame == self.frame
            && made.order == self.order
            && made.removed == self.removed
            && made.groups == self.groups;
        if same {
            Arc::clone(self)
        } else {
            Arc::new(made)
        }
    }

    /// What it holds besides itself, bytes (an estimate: what the budget counts).
    fn weight(&self) -> usize {
        let group = |g: &GroupRecord| std::mem::size_of::<GroupRecord>() + g.id.len();
        std::mem::size_of::<PageRecord>()
            + self.groups.iter().map(group).sum::<usize>()
            + self.removed.iter().map(|(id, _)| id.len() + 32).sum::<usize>()
    }

    /// Its tombstones at or below `floor` dropped.
    fn pruned(&self, floor: u64) -> Option<PageRecord> {
        if !self.removed.iter().any(|(_, at)| *at <= floor) {
            return None;
        }
        let mut out = self.clone();
        out.removed.retain(|(_, at)| *at > floor);
        Some(out)
    }
}

/// The pages of the version the feed last wrote, as records.
#[derive(Debug, Clone)]
pub(crate) struct PageRecords {
    /// The seq they are of.
    pub(crate) seq: u64,
    /// Below this, a `since` is not answered exactly (its tombstones are gone).
    pub(crate) floor: u64,
    /// The seq the page list (its count or a frame) last changed at.
    pub(crate) list: u64,
    /// Each page's record.
    pub(crate) pages: Vec<Arc<PageRecord>>,
}

/// What changed on one page after a `since`: the groups whose content changed (or came),
/// those only moved, those removed, and its order where it changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PageSince {
    /// The page (0-based).
    pub(crate) page: usize,
    /// The groups to send whole, by id.
    pub(crate) groups: Vec<String>,
    /// The groups only moved: id and offset.
    pub(crate) moved: Vec<(String, i64)>,
    /// The groups removed.
    pub(crate) removed: Vec<String>,
    /// The page's order, where it changed.
    pub(crate) order: Option<Vec<String>>,
}

impl PageRecords {
    /// The pages `pages` first written at `seq`: the floor is `seq`.
    pub(crate) fn first(pages: &[FeedPage], seq: u64) -> PageRecords {
        PageRecords {
            seq,
            floor: seq,
            list: seq,
            pages: pages.iter().map(|p| Arc::new(PageRecord::new(p, seq))).collect(),
        }
    }

    /// These records advanced to `seq`, the version whose page count is `count`, its pages
    /// `written` written again (by 0-based page): every other page is the same page as
    /// before (a page past the last before is always written).
    pub(crate) fn advanced(&self, seq: u64, count: usize, written: &BTreeMap<usize, FeedPage>) -> PageRecords {
        let mut list = self.list;
        if count != self.pages.len() {
            list = seq;
        }
        let pages = (0..count)
            .filter_map(|k| match (self.pages.get(k), written.get(&k)) {
                (Some(was), Some(page)) => {
                    if was.frame != page.frame {
                        list = seq;
                    }
                    Some(was.again(page, seq))
                }
                (Some(was), None) => Some(Arc::clone(was)),
                (None, Some(page)) => Some(Arc::new(PageRecord::new(page, seq))),
                (None, None) => None,
            })
            .collect();
        PageRecords {
            seq,
            floor: self.floor,
            list,
            pages,
        }
    }

    /// Whether `since` is answered exactly: at or above the floor, at or below these.
    pub(crate) fn answers(&self, since: u64) -> bool {
        (self.floor..=self.seq).contains(&since)
    }

    /// Whether the page list changed after `since`.
    pub(crate) fn list_since(&self, since: u64) -> bool {
        self.list > since
    }

    /// What changed on each page after `since` (every group of every page where `since` is
    /// `None`: a full answer).
    pub(crate) fn since(&self, since: Option<u64>) -> Vec<PageSince> {
        let after = |at: u64| since.is_none_or(|s| at > s);
        self.pages
            .iter()
            .enumerate()
            .filter(|(_, p)| after(p.changed))
            .map(|(page, p)| {
                let mut out = PageSince {
                    page,
                    ..PageSince::default()
                };
                for g in &p.groups {
                    if after(g.changed) {
                        out.groups.push(g.id.clone());
                    } else if after(g.moved) {
                        out.moved.push((g.id.clone(), g.dy));
                    }
                }
                if since.is_some() {
                    out.removed = p
                        .removed
                        .iter()
                        .filter(|(_, at)| after(*at))
                        .map(|(id, _)| id.clone())
                        .collect();
                }
                if after(p.order) {
                    out.order = Some(p.groups.iter().map(|g| g.id.clone()).collect());
                }
                out
            })
            .collect()
    }

    /// What they hold, bytes (an estimate: what the budget counts). A page shared with the
    /// records before is counted in each.
    pub(crate) fn weight(&self) -> usize {
        std::mem::size_of::<PageRecords>() + self.pages.iter().map(|p| p.weight() + 16).sum::<usize>()
    }

    /// Their tombstones dropped: the floor raised to these records' seq; `None` where they
    /// hold none.
    pub(crate) fn pruned(&self) -> Option<PageRecords> {
        let mut any = false;
        let pages = self
            .pages
            .iter()
            .map(|p| match p.pruned(self.seq) {
                Some(made) => {
                    any = true;
                    Arc::new(made)
                }
                None => Arc::clone(p),
            })
            .collect();
        any.then_some(PageRecords {
            seq: self.seq,
            floor: self.seq,
            list: self.list,
            pages,
        })
    }
}

/// When a block came, last changed and went, by seq.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BlockSeqs {
    pub(crate) came: Option<u64>,
    pub(crate) changed: u64,
    pub(crate) gone: Option<u64>,
}

/// A version's parts and blocks as the feed last saw them change.
#[derive(Debug, Clone)]
pub(crate) struct DocRecords {
    /// The seq they are of.
    pub(crate) seq: u64,
    /// The state number of that version.
    pub(crate) state: u64,
    /// Below this, a `since` is not answered exactly.
    pub(crate) floor: u64,
    /// Each part that changed since the feed began, with the seq it last changed at.
    pub(crate) parts: HashMap<String, u64>,
    /// Each block that changed since the feed began, by id.
    pub(crate) blocks: HashMap<String, BlockSeqs>,
}

/// What changed after a `since`: the parts, and the blocks changed, come and gone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockChanges {
    /// The blocks both hold whose content changed, by id, sorted.
    pub changed: Vec<String>,
    /// The blocks that came, by id, sorted.
    pub came: Vec<String>,
    /// The blocks that went, by the id they had, sorted.
    pub gone: Vec<String>,
}

/// One version's change against the one before, as the records take it: its parts, and its
/// blocks by id.
#[derive(Debug, Clone, Default)]
pub(crate) struct DocStep {
    pub(crate) parts: Vec<String>,
    pub(crate) changed: Vec<String>,
    pub(crate) came: Vec<String>,
    pub(crate) gone: Vec<String>,
}

impl DocRecords {
    /// The records of a version first seen at `seq` (numbered `state`): the floor is `seq`.
    pub(crate) fn first(seq: u64, state: u64) -> DocRecords {
        DocRecords {
            seq,
            state,
            floor: seq,
            parts: HashMap::new(),
            blocks: HashMap::new(),
        }
    }

    /// These records advanced to `seq` (numbered `state`) by `step`.
    pub(crate) fn advanced(&self, seq: u64, state: u64, step: &DocStep) -> DocRecords {
        let mut out = self.clone();
        out.seq = seq;
        out.state = state;
        for part in &step.parts {
            out.parts.insert(part.clone(), seq);
        }
        for id in &step.changed {
            out.blocks
                .entry(id.clone())
                .and_modify(|b| b.changed = seq)
                .or_insert(BlockSeqs {
                    came: None,
                    changed: seq,
                    gone: None,
                });
        }
        for id in &step.came {
            out.blocks.insert(
                id.clone(),
                BlockSeqs {
                    came: Some(seq),
                    changed: seq,
                    gone: None,
                },
            );
        }
        for id in &step.gone {
            out.blocks
                .entry(id.clone())
                .and_modify(|b| {
                    b.gone = Some(seq);
                    b.changed = seq;
                })
                .or_insert(BlockSeqs {
                    came: None,
                    changed: seq,
                    gone: Some(seq),
                });
        }
        out
    }

    /// Whether `since` is answered exactly.
    pub(crate) fn answers(&self, since: u64) -> bool {
        (self.floor..=self.seq).contains(&since)
    }

    /// The parts changed after `since`, sorted.
    pub(crate) fn parts_since(&self, since: u64) -> Vec<String> {
        let mut out: Vec<String> = self
            .parts
            .iter()
            .filter(|(_, at)| **at > since)
            .map(|(p, _)| p.clone())
            .collect();
        out.sort();
        out
    }

    /// The blocks changed, come and gone after `since`: a block that came after it and went
    /// is in none; one that came after it is in `came` only.
    pub(crate) fn blocks_since(&self, since: u64) -> BlockChanges {
        let mut out = BlockChanges::default();
        for (id, b) in &self.blocks {
            if b.changed <= since {
                continue;
            }
            let came = b.came.is_some_and(|c| c > since);
            match (came, b.gone) {
                (true, Some(_)) => {}
                (true, None) => out.came.push(id.clone()),
                (false, Some(_)) => out.gone.push(id.clone()),
                (false, None) => out.changed.push(id.clone()),
            }
        }
        out.changed.sort();
        out.came.sort();
        out.gone.sort();
        out
    }

    /// What they hold, bytes (an estimate).
    pub(crate) fn weight(&self) -> usize {
        std::mem::size_of::<DocRecords>()
            + self.parts.keys().map(|p| p.len() + 40).sum::<usize>()
            + self.blocks.keys().map(|id| id.len() + 64).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn frame() -> FeedFrame {
        FeedFrame {
            width: "10pt".into(),
            height: "10pt".into(),
            width_px: 10,
            height_px: 10,
        }
    }

    fn group(id: &str, dy: i64, content: &str) -> FeedGroup {
        FeedGroup {
            id: id.into(),
            dy,
            content: content.into(),
            defs: String::new(),
            pictures: Vec::new(),
        }
    }

    fn page(groups: Vec<FeedGroup>) -> FeedPage {
        FeedPage { frame: frame(), groups }
    }

    #[test]
    fn a_group_only_moved_is_moved_and_one_gone_is_removed() {
        let first = PageRecords::first(&[page(vec![group("a", 0, "x"), group("b", 5, "y")])], 1);
        let mut written = BTreeMap::new();
        written.insert(0, page(vec![group("b", 9, "y"), group("c", 2, "z")]));
        let now = first.advanced(2, 1, &written);
        let since = now.since(Some(1));
        assert_eq!(since.len(), 1);
        assert_eq!(since[0].groups, vec!["c".to_owned()]);
        assert_eq!(since[0].moved, vec![("b".to_owned(), 9)]);
        assert_eq!(since[0].removed, vec!["a".to_owned()]);
        assert_eq!(since[0].order, Some(vec!["b".to_owned(), "c".to_owned()]));
        assert!(now.since(Some(2)).is_empty());
        let pruned = now.pruned().unwrap();
        assert!(!pruned.answers(1) && pruned.answers(2));
    }

    #[test]
    fn a_page_written_the_same_is_kept_as_it_was() {
        let first = PageRecords::first(&[page(vec![group("a", 0, "x")])], 1);
        let mut written = BTreeMap::new();
        written.insert(0, page(vec![group("a", 0, "x")]));
        let now = first.advanced(2, 1, &written);
        assert!(Arc::ptr_eq(&now.pages[0], &first.pages[0]));
        assert!(now.since(Some(1)).is_empty());
    }

    #[test]
    fn blocks_that_came_and_went_after_since_are_in_neither() {
        let doc = DocRecords::first(1, 1);
        let step = DocStep {
            parts: vec!["word/document.xml".into()],
            changed: vec!["p:1".into()],
            came: vec!["p:2".into()],
            gone: vec![],
        };
        let doc = doc.advanced(2, 2, &step);
        let doc = doc.advanced(
            3,
            3,
            &DocStep {
                gone: vec!["p:2".into(), "p:3".into()],
                ..DocStep::default()
            },
        );
        let found = doc.blocks_since(1);
        assert_eq!(found.changed, vec!["p:1".to_owned()]);
        assert!(found.came.is_empty());
        assert_eq!(found.gone, vec!["p:3".to_owned()]);
        assert_eq!(doc.blocks_since(2).gone, vec!["p:2".to_owned(), "p:3".to_owned()]);
        assert_eq!(doc.parts_since(1), vec!["word/document.xml".to_owned()]);
    }
}
