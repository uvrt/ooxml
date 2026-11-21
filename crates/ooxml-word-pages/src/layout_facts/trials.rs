//! Owns: what a range's keep set or released does to the pages, read from a trial layout
//! of the document with that one change (docx-agent's `keep_trials._trial`, round 40): where
//! the range then starts and ends, how much of the page before it is left blank (then and
//! now), the document's page count now and then, and the page what follows starts on now
//! and then ([`KeepEffect::of`]). The blocks are found by their layout paths, which a keep
//! (a paragraph's properties) does not change.
//!
//! Must not: lay out (the engine makes the trial, [`crate::PageEngine::trial`]), word a
//! reply, or change the document.

use super::ranges::room;
use super::tree::block_keys_top;
use super::Facts;
use ooxml_word_layout::facts::{LayoutFacts, PageFacts};
use std::collections::HashSet;

/// The blank at the foot of the page before a range kept, then and now.
#[derive(Debug, Clone, PartialEq)]
pub struct Blank {
    /// The 1-based page.
    pub page: usize,
    /// Its blank in the trial (`on`) or now (released), points.
    pub pt: f64,
    /// That blank as a share of the body's height there.
    pub share: f64,
    /// Its blank on the other side, points.
    pub now_pt: f64,
    /// That blank's share.
    pub now_share: f64,
}

/// What one trial found.
#[derive(Debug, Clone, PartialEq)]
pub struct KeepEffect {
    /// The 1-based page the range then starts on.
    pub starts: usize,
    /// The 1-based page it then ends on.
    pub ends: usize,
    /// The page before where it starts kept, where it starts later kept than free.
    pub blank: Option<Blank>,
    /// The document's page count now and then.
    pub document_pages: (usize, usize),
    /// The page what follows the range starts on now and then.
    pub after: Option<(usize, usize)>,
}

/// The range's top-level blocks among the layout's, by path, and the block that follows
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialBlocks {
    keys: Vec<String>,
    after: Option<String>,
}

impl Facts<'_> {
    /// The layout paths of the body blocks `ids` (a range's, as a keep's call names
    /// them) and the top-level block drawn first after the last of them: `None` where one
    /// of them is not a top-level block the layout draws.
    #[must_use]
    pub fn trial_blocks(&self, ids: &[String]) -> Option<TrialBlocks> {
        let mut order: Vec<&str> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for page in &self.version().facts().pages {
            for key in &page.blocks {
                if block_keys_top(key) && seen.insert(key) {
                    order.push(key);
                }
            }
        }
        let keys: Vec<String> = ids
            .iter()
            .map(|id| {
                let path = self.version().path_of(id)?;
                seen.contains(&*path).then(|| path.to_string())
            })
            .collect::<Option<_>>()?;
        let mine: HashSet<&str> = keys.iter().map(String::as_str).collect();
        let last = keys.iter().filter_map(|k| order.iter().position(|o| o == k)).max()?;
        let after = order
            .iter()
            .skip(last + 1)
            .find(|k| !mine.contains(**k))
            .map(|k| (*k).to_owned());
        Some(TrialBlocks { keys, after })
    }
}

/// Where some top-level blocks are drawn in one layout: their first and last 1-based page,
/// and the first page of the block after them.
struct Placed {
    start: Option<usize>,
    end: Option<usize>,
    after: Option<usize>,
}

impl Placed {
    fn of(pages: &[PageFacts], blocks: &TrialBlocks) -> Placed {
        let wanted: HashSet<&str> = blocks.keys.iter().map(String::as_str).collect();
        let drawn: Vec<usize> = pages
            .iter()
            .enumerate()
            .filter(|(_, p)| p.blocks.iter().any(|b| wanted.contains(&**b)))
            .map(|(k, _)| k + 1)
            .collect();
        let after = blocks.after.as_deref().and_then(|after| {
            pages
                .iter()
                .position(|p| p.blocks.iter().any(|b| **b == *after))
                .map(|k| k + 1)
        });
        Placed {
            start: drawn.first().copied(),
            end: drawn.last().copied(),
            after,
        }
    }
}

impl KeepEffect {
    /// What keeping (`on`) or releasing `blocks` did, `now` the layout checked and `then`
    /// the trial's: `None` where the range is drawn in neither.
    #[must_use]
    pub fn of(now: &LayoutFacts, then: &LayoutFacts, blocks: &TrialBlocks, on: bool) -> Option<KeepEffect> {
        let (was, will) = (Placed::of(&now.pages, blocks), Placed::of(&then.pages, blocks));
        let (now_start, then_start) = (was.start?, will.start?);
        let ends = will.end?;
        let (kept_start, free_start) = if on {
            (then_start, now_start)
        } else {
            (now_start, then_start)
        };
        let mut blank = None;
        if kept_start > free_start && kept_start > 1 {
            let page = kept_start - 1;
            let (kept_pages, free_pages) = if on {
                (&then.pages, &now.pages)
            } else {
                (&now.pages, &then.pages)
            };
            let kept_room = room(kept_pages.get(page - 1)?);
            let free_room = room(free_pages.get(page - 1)?);
            let (room_then, room_now) = if on {
                (kept_room, free_room)
            } else {
                (free_room, kept_room)
            };
            blank = Some(Blank {
                page,
                pt: room_then.0,
                share: room_then.1,
                now_pt: room_now.0,
                now_share: room_now.1,
            });
        }
        let after = match (&blocks.after, was.after, will.after) {
            (Some(_), Some(a), Some(b)) => Some((a, b)),
            _ => None,
        };
        Some(KeepEffect {
            starts: then_start,
            ends,
            blank,
            document_pages: (now.pages.len(), then.pages.len()),
            after,
        })
    }
}
