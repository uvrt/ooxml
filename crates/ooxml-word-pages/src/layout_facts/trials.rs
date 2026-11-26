//! Owns: what a range's keep set or released does to the pages, read from a trial layout
//! of the document with that one change (docx-agent's `keep_trials._trial`, round 40): where
//! the range then starts and ends, how much of the page before it is left blank (then and
//! now), the document's page count now and then, and the page what follows starts on now
//! and then ([`KeepEffect::of`]). The blocks are named across the two layouts by their
//! layout paths, which a keep (a paragraph's properties) does not change -- their keys may
//! (the trial's edit stamps an unstamped document) -- and found in each by key. The blank is measured with the lines the
//! facts leave out -- the page's text boxes' and footnotes' ([`BeyondFacts`]), which
//! docx-agent counts -- asked of the caller for the two pages measured only ([`Side`]).
//!
//! Must not: lay out (the engine makes the trial, [`crate::PageEngine::trial`]), word a
//! reply, or change the document.

use super::ranges::{room, BeyondFacts};
use super::tree::is_top;
use super::Facts;
use ooxml_word_layout::facts::{LayoutFacts, PageFacts};
use ooxml_word_model::docx::VersionNames;
use ooxml_word_model::ids::{Anchor, StoryIx};
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

/// Which layout a page measured is of: the one checked, or the trial's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The layout checked.
    Now,
    /// The trial's.
    Then,
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
        let names = self.names();
        let mut order: Vec<Anchor> = Vec::new();
        let mut seen: HashSet<Anchor> = HashSet::new();
        for page in &self.version().facts().pages {
            for block in &page.blocks {
                if is_top(names, *block) && seen.insert(*block) {
                    order.push(*block);
                }
            }
        }
        let mine: Vec<Anchor> = ids
            .iter()
            .map(|id| {
                let block = Anchor::whole(names.key_of_id(StoryIx::BODY, id)?);
                seen.contains(&block).then_some(block)
            })
            .collect::<Option<_>>()?;
        let last = mine.iter().filter_map(|k| order.iter().position(|o| o == k)).max()?;
        let after = order.iter().skip(last + 1).find(|k| !mine.contains(k));
        Some(TrialBlocks {
            keys: mine.iter().map(|b| names.path(*b)).collect::<Option<_>>()?,
            after: match after {
                Some(after) => Some(names.path(*after)?),
                None => None,
            },
        })
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
    /// Where `blocks` (named by path) are drawn in a layout whose facts are `facts`, each
    /// found there by key through its version's `names`.
    fn of((facts, names): (&LayoutFacts, &VersionNames), blocks: &TrialBlocks) -> Placed {
        let key = |path: &str| names.key_at_path(StoryIx::BODY, path).map(Anchor::whole);
        let wanted: HashSet<Anchor> = blocks.keys.iter().filter_map(|p| key(p)).collect();
        let pages = &facts.pages;
        let drawn: Vec<usize> = pages
            .iter()
            .enumerate()
            .filter(|(_, p)| p.blocks.iter().any(|b| wanted.contains(b)))
            .map(|(k, _)| k + 1)
            .collect();
        let after = blocks
            .after
            .as_deref()
            .and_then(key)
            .and_then(|after| pages.iter().position(|p| p.blocks.contains(&after)).map(|k| k + 1));
        Placed {
            start: drawn.first().copied(),
            end: drawn.last().copied(),
            after,
        }
    }
}

impl KeepEffect {
    /// What keeping (`on`) or releasing `blocks` did, `now` the layout checked and `then`
    /// the trial's (each its facts and its version's names): `None` where the range is drawn in neither. `foot` gives what a page
    /// (of that side's layout) draws beyond its facts, asked only of the two pages whose
    /// blank is measured.
    pub fn of(
        (now, then): ((&LayoutFacts, &VersionNames), (&LayoutFacts, &VersionNames)),
        blocks: &TrialBlocks,
        on: bool,
        foot: &mut dyn FnMut(Side, &PageFacts) -> BeyondFacts,
    ) -> Option<KeepEffect> {
        let (was, will) = (Placed::of(now, blocks), Placed::of(then, blocks));
        let (now, then) = (now.0, then.0);
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
            let measured = |side: Side, foot: &mut dyn FnMut(Side, &PageFacts) -> BeyondFacts| {
                let pages = if side == Side::Then { &then.pages } else { &now.pages };
                let at = pages.get(page - 1)?;
                Some(room(at, &foot(side, at)))
            };
            let room_then = measured(Side::Then, foot)?;
            let room_now = measured(Side::Now, foot)?;
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
