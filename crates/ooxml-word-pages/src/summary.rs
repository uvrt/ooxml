//! Owns: a version's layout summary ([`Summary`], docx-agent's `LayoutSummary`): per page
//! a digest of what it draws besides its lines, and per line its block's id (the tool's,
//! in the version's bytes; a story's line by its path), whether it is the body's, and a
//! digest of what it draws; and the comparison of two summaries ([`Summary::compare`],
//! `LayoutSummary.compare`): the pages that differ and why, the blocks whose first page
//! moved, ids followed through the caller's aliases -- made of the parts a comparison of
//! two versions page by page makes too ([`page_change`], [`first_pages`], [`reflow_of`]),
//! so that both answer alike.
//!
//! Must not: paint (the engine paints a page for its digests), or follow an alias
//! anywhere but in a comparison.

use ooxml_base::Digest;
use ooxml_word_model::docx::VersionNames;
use ooxml_word_model::ids::{Anchor, StoryIx};
use ooxml_word_model::value::Text;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// One line of a page's summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryLine {
    /// Its paragraph's id, where it is a body paragraph the version names.
    pub id: Option<Text>,
    /// Its paragraph's path, where it has no id (a header's, footer's or note's line, a
    /// separator the layout makes): a body block's path moves where its id does not, and a
    /// page's summary is kept across versions by its blocks' keys.
    pub path: Option<Text>,
    /// It is the body's (not a header's, footer's or note's).
    pub body: bool,
    /// What it draws, digested.
    pub drawn: Digest,
}

/// One page's summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSummary {
    /// What it draws besides its lines, digested.
    pub rest: Digest,
    /// Its lines, in drawing order.
    pub lines: Vec<SummaryLine>,
}

/// A version's summary.
#[derive(Debug, Clone)]
pub struct Summary {
    /// Every page's.
    pub pages: Vec<Arc<PageSummary>>,
    /// The pages known (all of them: a layout that stops is a fallback).
    pub known: usize,
    /// The names of the version it was made of: its body lines' paths are named through
    /// them at the edge ([`Summary::path_of`]).
    pub names: Arc<VersionNames>,
}

/// What changed between two summaries (`Reflow`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reflow {
    /// 1-based pages whose drawing differs, among the pages both know.
    pub changed: Vec<usize>,
    /// The page count before and after.
    pub page_count: (usize, usize),
    /// Blocks whose first page changed: (id, (before, after)), in the order this version
    /// first draws them.
    pub moved: Vec<(String, (usize, usize))>,
    /// Pages past either's last known page.
    pub unknown: Vec<usize>,
    /// Why each changed page changed: `content`, `flow`, `stories`, `added`, `removed`.
    pub why: BTreeMap<usize, &'static str>,
}

/// An id followed through `aliases` to its end.
pub(crate) fn canon<'a>(aliases: &'a HashMap<String, String>, id: &'a str) -> &'a str {
    let mut at = id;
    let mut steps = 0;
    while let Some(next) = aliases.get(at) {
        steps += 1;
        if steps > aliases.len() || next == id {
            break;
        }
        at = next;
    }
    at
}

/// What a page shows for a comparison: its rest and its lines (`all`, or the body's).
fn content<'a>(page: &'a PageSummary, body_only: bool, aliases: &'a HashMap<String, String>) -> Vec<(&'a str, Digest)> {
    page.lines
        .iter()
        .filter(|l| !body_only || l.body)
        .map(|l| {
            let key = match (l.id.as_deref(), l.path.as_deref()) {
                (Some(id), _) => canon(aliases, id),
                (None, path) => path.unwrap_or_default(),
            };
            (key, l.drawn)
        })
        .collect()
}

/// The body's ids a page draws, followed through `aliases`.
fn page_ids<'a>(page: &'a PageSummary, aliases: &'a HashMap<String, String>) -> Vec<&'a str> {
    page.lines
        .iter()
        .filter(|l| l.body)
        .filter_map(|l| l.id.as_deref())
        .map(|id| canon(aliases, id))
        .collect()
}

/// How page `old` became `new` (`LayoutSummary.compare`'s reason): `None` where it draws
/// the same, else `stories` (only its headers, footers or notes changed), `content` (the
/// same body blocks, drawn otherwise) or `flow` (other blocks).
pub(crate) fn page_change(
    old: &PageSummary,
    new: &PageSummary,
    aliases: &HashMap<String, String>,
) -> Option<&'static str> {
    if old.rest == new.rest && content(old, false, aliases) == content(new, false, aliases) {
        return None;
    }
    if old.rest == new.rest && content(old, true, aliases) == content(new, true, aliases) {
        return Some("stories");
    }
    let mut a = page_ids(old, aliases);
    let mut b = page_ids(new, aliases);
    a.sort_unstable();
    a.dedup();
    b.sort_unstable();
    b.dedup();
    Some(if a == b { "content" } else { "flow" })
}

/// Each body id's first 1-based page, in the order first drawn, from each page's body ids
/// (followed through the aliases already).
pub(crate) fn first_pages<'a, P>(pages: impl Iterator<Item = P>) -> Vec<(&'a str, usize)>
where
    P: IntoIterator<Item = &'a str>,
{
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (index, ids) in pages.enumerate() {
        for id in ids {
            if seen.insert(id) {
                out.push((id, index + 1));
            }
        }
    }
    out
}

/// The two sides of a comparison: each one's pages and pages known.
pub(crate) struct Counts {
    pub(crate) before: (usize, usize),
    pub(crate) now: (usize, usize),
}

/// What changed between two versions (`LayoutSummary.compare`), from their `counts`, how
/// each page both know changed (`change`, by 1-based number: `None` where one of them has
/// no such page, `Some(None)` where it draws the same) and each one's first pages.
pub(crate) fn reflow_of(
    counts: &Counts,
    mut change: impl FnMut(usize) -> Option<Option<&'static str>>,
    first_before: &[(&str, usize)],
    first_now: &[(&str, usize)],
) -> Reflow {
    let limit = counts.before.1.min(counts.now.1);
    let count = counts.before.0.max(counts.now.0);
    let mut out = Reflow {
        page_count: (counts.before.0, counts.now.0),
        ..Reflow::default()
    };
    for number in 1..=count.min(limit) {
        match change(number) {
            None => {
                out.changed.push(number);
                out.why
                    .insert(number, if number > counts.before.0 { "added" } else { "removed" });
            }
            Some(None) => {}
            Some(Some(why)) => {
                out.changed.push(number);
                out.why.insert(number, why);
            }
        }
    }
    out.unknown = (limit + 1..=count).collect();
    let before: HashMap<&str, usize> = first_before.iter().copied().collect();
    for (id, page) in first_now {
        if let Some(was) = before.get(id).filter(|was| **was != *page) {
            out.moved.push(((*id).to_owned(), (*was, *page)));
        }
    }
    out
}

impl Summary {
    /// The path of `line`'s paragraph (a line of one of its pages): its own where it has no
    /// id, else its block's, named through the version's names.
    #[must_use]
    pub fn path_of(&self, line: &SummaryLine) -> Option<String> {
        if let Some(path) = &line.path {
            return Some(path.to_string());
        }
        let key = self.names.key_of_id(StoryIx::BODY, line.id.as_deref()?)?;
        self.names.path(Anchor::whole(key))
    }

    /// Its pages.
    pub fn count(&self) -> usize {
        self.pages.len()
    }

    /// Which pages of this summary differ from `before`'s (`LayoutSummary.compare`), ids
    /// followed through `aliases`.
    pub fn compare(&self, before: &Summary, aliases: &HashMap<String, String>) -> Reflow {
        let counts = Counts {
            before: (before.count(), before.known),
            now: (self.count(), self.known),
        };
        let change = |number: usize| {
            let (old, new) = (before.pages.get(number - 1)?, self.pages.get(number - 1)?);
            Some(if Arc::ptr_eq(old, new) {
                None
            } else {
                page_change(old, new, aliases)
            })
        };
        let first_before = first_pages(before.pages.iter().map(|p| page_ids(p, aliases)));
        let first_now = first_pages(self.pages.iter().map(|p| page_ids(p, aliases)));
        reflow_of(&counts, change, &first_before, &first_now)
    }
}

/// The paths a header's, footer's or note's lines start with: a line of none of them,
/// drawn with no story, is the body's.
const STORIES: [&str; 4] = ["w:footnote[", "w:endnote[", "w:hdr", "w:ftr"];

/// Whether a line of `path`, of `story` (a header's or footer's), is the body's.
fn in_body(path: &str, story: bool) -> bool {
    !story && !STORIES.iter().any(|p| path.starts_with(p))
}

/// A page's summary from its digests: each body line named by `version`'s id for it, the
/// rest by their paths.
pub(crate) fn page_summary(digests: crate::engine::PageDigests, version: &crate::version::LaidVersion) -> PageSummary {
    let lines = digests
        .lines
        .into_iter()
        .map(|(path, story, drawn)| {
            let body = in_body(&path, story.is_some());
            let id = if body {
                version.id_of(&path).map(|id| Text::from(id.as_str()))
            } else {
                None
            };
            SummaryLine {
                path: id.is_none().then(|| Text::from(path.as_str())),
                id,
                body,
                drawn,
            }
        })
        .collect();
    PageSummary {
        rest: digests.rest,
        lines,
    }
}
