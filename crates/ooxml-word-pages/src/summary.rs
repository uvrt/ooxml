//! Owns: a version's layout summary ([`Summary`], docx-agent's `LayoutSummary`): per page
//! a digest of what it draws besides its lines, and per line its block's id (the tool's,
//! in the version's bytes; a story's line by its path), whether it is the body's, and a
//! digest of what it draws; and the comparison of two summaries ([`Summary::compare`],
//! `LayoutSummary.compare`): the pages that differ and why, the blocks whose first page
//! moved, ids followed through the caller's aliases.
//!
//! Each page's part is kept by the page as laid out ([`SummaryMemo`]) and taken as it was
//! for a page the next version's relayout kept with the same items, page count, sections'
//! page counts and field values: an edit digests the pages it reached (lane L4b).
//!
//! Must not: paint (the engine paints a page for its digests), or follow an alias
//! anywhere but in a comparison.

use ooxml_base::Digest;
use ooxml_word_layout::flow::FlowItem;
use ooxml_word_layout::paginate::LaidPage;
use ooxml_word_model::value::Text;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// One line of a page's summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryLine {
    /// Its paragraph's id, where it is a body paragraph the version names.
    pub id: Option<Text>,
    /// Its paragraph's path.
    pub path: Text,
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
fn canon<'a>(aliases: &'a HashMap<String, String>, id: &'a str) -> &'a str {
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
            let key = l.id.as_deref().map_or(&*l.path, |id| canon(aliases, id));
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

impl Summary {
    /// Its pages.
    pub fn count(&self) -> usize {
        self.pages.len()
    }

    /// Each body id's first 1-based page, in the order first drawn.
    fn first_pages<'a>(&'a self, aliases: &'a HashMap<String, String>) -> Vec<(&'a str, usize)> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for (index, page) in self.pages.iter().enumerate() {
            for id in page_ids(page, aliases) {
                if seen.insert(id) {
                    out.push((id, index + 1));
                }
            }
        }
        out
    }

    /// Which pages of this summary differ from `before`'s (`LayoutSummary.compare`), ids
    /// followed through `aliases`.
    pub fn compare(&self, before: &Summary, aliases: &HashMap<String, String>) -> Reflow {
        let limit = before.known.min(self.known);
        let count = before.count().max(self.count());
        let mut out = Reflow {
            page_count: (before.count(), self.count()),
            ..Reflow::default()
        };
        for number in 1..=count.min(limit) {
            let (Some(old), Some(new)) = (before.pages.get(number - 1), self.pages.get(number - 1)) else {
                out.changed.push(number);
                out.why
                    .insert(number, if number > before.count() { "added" } else { "removed" });
                continue;
            };
            if Arc::ptr_eq(old, new)
                || (old.rest == new.rest && content(old, false, aliases) == content(new, false, aliases))
            {
                continue;
            }
            out.changed.push(number);
            let why = if old.rest == new.rest && content(old, true, aliases) == content(new, true, aliases) {
                "stories"
            } else {
                let mut a = page_ids(old, aliases);
                let mut b = page_ids(new, aliases);
                a.sort_unstable();
                a.dedup();
                b.sort_unstable();
                b.dedup();
                if a == b {
                    "content"
                } else {
                    "flow"
                }
            };
            out.why.insert(number, why);
        }
        out.unknown = (limit + 1..=count).collect();
        let first_before: HashMap<&str, usize> = before.first_pages(aliases).into_iter().collect();
        for (id, page) in self.first_pages(aliases) {
            if let Some(was) = first_before.get(id).filter(|was| **was != page) {
                out.moved.push((id.to_owned(), (*was, page)));
            }
        }
        out
    }
}

/// What every page's summary of `layout` depends on beyond the page itself: the page
/// count (`NUMPAGES`), each section's page count (`SECTIONPAGES`) and the page-number
/// fields' values, digested under `key`.
pub(crate) fn context_of(layout: &ooxml_word_layout::DocumentLayout, key: ooxml_base::DigestKey) -> Digest {
    let mut sections: Vec<usize> = Vec::new();
    for info in &layout.pagination.info {
        if sections.len() <= info.section {
            sections.resize(info.section + 1, 0);
        }
        if let Some(count) = sections.get_mut(info.section) {
            *count += 1;
        }
    }
    Digest::of(
        key,
        format!("{:?}", (layout.page_count(), sections, &layout.fields)).as_bytes(),
    )
}

/// A page's summary from its digests: each body line named by `version`'s id for it.
pub(crate) fn page_summary(digests: crate::engine::PageDigests, version: &crate::version::LaidVersion) -> PageSummary {
    const STORIES: [&str; 4] = ["w:footnote[", "w:endnote[", "w:hdr", "w:ftr"];
    let lines = digests
        .lines
        .into_iter()
        .map(|(path, story, drawn)| {
            let body = story.is_none() && !STORIES.iter().any(|p| path.starts_with(p));
            SummaryLine {
                id: if body {
                    version.ids.all().get(&path).cloned()
                } else {
                    None
                },
                path: Text::from(path.as_str()),
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

/// What a page's summary was made of.
struct Kept {
    page: Arc<LaidPage>,
    items: Vec<FlowItem>,
    context: Digest,
    summary: Arc<PageSummary>,
}

/// Each page's summary in the version summarised last, by its laid page.
#[derive(Default)]
pub struct SummaryMemo {
    pages: HashMap<usize, Kept>,
}

impl std::fmt::Debug for SummaryMemo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SummaryMemo").field("pages", &self.pages.len()).finish()
    }
}

/// A page about to be summarised: its laid page and items, where it has a laid page.
pub(crate) struct PageKey {
    pub(crate) page: Arc<LaidPage>,
    pub(crate) items: Vec<FlowItem>,
}

impl SummaryMemo {
    /// The summary of page `key` kept from the last version, where its page, items and
    /// `context` are the same.
    pub(crate) fn take(&mut self, key: &PageKey, context: Digest) -> Option<Arc<PageSummary>> {
        let ptr = Arc::as_ptr(&key.page) as usize;
        let kept = self.pages.get(&ptr)?;
        (Arc::ptr_eq(&kept.page, &key.page)
            && kept.context == context
            && crate::page_said::same_items(&kept.items, &key.items))
        .then(|| Arc::clone(&kept.summary))
    }

    /// Keep `summaries` (each page's, with its key) as the version summarised last.
    pub(crate) fn keep(&mut self, summaries: Vec<(PageKey, Arc<PageSummary>)>, context: Digest) {
        self.pages = summaries
            .into_iter()
            .map(|(key, summary)| {
                (
                    Arc::as_ptr(&key.page) as usize,
                    Kept {
                        page: key.page,
                        items: key.items,
                        context,
                        summary,
                    },
                )
            })
            .collect();
    }
}
