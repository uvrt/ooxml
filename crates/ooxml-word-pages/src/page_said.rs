//! Owns: what each page's body and drawings said in the version laid out last
//! ([`PageMemo`]), kept by the page as laid out -- its `LaidPage` and the flow items it
//! draws, by pointer -- so that a page the next version's relayout kept is said again
//! without being walked again (lane L4b: a 3,000-page version walked every page for its
//! warnings, 16 s an edit). A page is taken as it was only where it is the very same laid
//! page drawing the very same items, with the same tables said of before it and the same
//! drawings in its stories; anything else is walked again.
//!
//! Must not: say anything itself (the warnings module does), or keep more than the pages
//! of the version laid out last.

use crate::warnings::Said;
use ooxml_word_layout::flow::FlowItem;
use ooxml_word_layout::paginate::LaidPage;
use ooxml_word_layout::DocumentLayout;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// One page as said: what it was said of, and what it said.
struct Kept {
    page: Arc<LaidPage>,
    items: Vec<FlowItem>,
    /// A story's drawing may be on any page (the walk's input).
    everywhere: bool,
    /// For each table on the page, whether it was said of on a page before.
    prior: Vec<bool>,
    said: Said,
    /// The tables first said of on this page.
    tables: Vec<usize>,
}

/// What each page of the version laid out last said, by its laid page.
#[derive(Default)]
pub struct PageMemo {
    pages: HashMap<usize, Kept>,
    stats: (u64, u64),
}

impl std::fmt::Debug for PageMemo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageMemo")
            .field("pages", &self.pages.len())
            .field("stats", &self.stats)
            .finish()
    }
}

impl PageMemo {
    /// A walk of a version's pages, taking what the last version's pages said.
    pub(crate) fn walk(&mut self) -> Walk {
        Walk {
            old: std::mem::take(&mut self.pages),
            new: HashMap::new(),
            tables: HashSet::new(),
            stats: (0, 0),
        }
    }

    /// Pages said again as they were, and pages walked, since the memo was made.
    pub fn stats(&self) -> (u64, u64) {
        self.stats
    }
}

/// One version's walk of its pages.
pub(crate) struct Walk {
    old: HashMap<usize, Kept>,
    new: HashMap<usize, Kept>,
    tables: HashSet<usize>,
    stats: (u64, u64),
}

/// What a page's body and drawings say, given the tables said of so far.
pub(crate) type PageBody<'b> = dyn FnMut(&mut Said, &mut HashSet<usize>) + 'b;

impl Walk {
    /// Say page `page` into `out`: as the last version's same page said, where it is the
    /// same, else by `body`.
    pub(crate) fn page(
        &mut self,
        layout: &DocumentLayout,
        (page, everywhere): (usize, bool),
        body: &mut PageBody<'_>,
        out: &mut Said,
    ) {
        let Some((laid, indices)) = laid_page(layout, page).map(|laid| (laid, item_indices(laid))) else {
            let mut local = Said::default();
            body(&mut local, &mut self.tables);
            out.extend(&local);
            return;
        };
        let items: Vec<FlowItem> = indices
            .iter()
            .filter_map(|i| layout.flow.items.get(*i).cloned())
            .collect();
        let tables_on: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|i| matches!(layout.flow.items.get(*i), Some(FlowItem::Table(_))))
            .collect();
        let prior: Vec<bool> = tables_on.iter().map(|t| self.tables.contains(t)).collect();
        let key = Arc::as_ptr(laid) as usize;
        let found = self.old.remove(&key).filter(|k| {
            Arc::ptr_eq(&k.page, laid) && k.everywhere == everywhere && k.prior == prior && same_items(&k.items, &items)
        });
        if let Some(kept) = found {
            out.extend(&kept.said);
            self.tables.extend(kept.tables.iter().copied());
            self.stats.0 += 1;
            self.new.insert(key, kept);
            return;
        }
        let mut local = Said::default();
        body(&mut local, &mut self.tables);
        let tables = tables_on
            .iter()
            .zip(&prior)
            .filter(|(t, before)| !**before && self.tables.contains(*t))
            .map(|(t, _)| *t)
            .collect();
        out.extend(&local);
        self.stats.1 += 1;
        self.new.insert(
            key,
            Kept {
                page: Arc::clone(laid),
                items,
                everywhere,
                prior,
                said: local,
                tables,
            },
        );
    }

    /// Keep what this version's pages said for the next.
    pub(crate) fn finish(self, memo: &mut PageMemo) {
        memo.pages = self.new;
        memo.stats.0 += self.stats.0;
        memo.stats.1 += self.stats.1;
    }
}

/// Page `page`'s laid page (0-based, blank pages counted), as the layout holds it.
fn laid_page(layout: &DocumentLayout, page: usize) -> Option<&Arc<LaidPage>> {
    let pages = &layout.pagination.pages;
    let k = pages.partition_point(|p| p.page < page);
    pages.get(k).filter(|p| p.page == page)
}

/// The flow items a laid page draws (its lines' and its rows'), each once, in order.
fn item_indices(laid: &LaidPage) -> Vec<usize> {
    let mut indices: Vec<usize> = laid
        .lines
        .iter()
        .map(|l| l.item)
        .chain(laid.rows.iter().map(|r| r.item))
        .collect();
    indices.sort_unstable();
    indices.dedup();
    indices
}

/// Page `page`'s laid page and the flow items it draws, where it has a laid page: what
/// a page's memo is kept by.
pub(crate) fn page_key(layout: &DocumentLayout, page: usize) -> Option<crate::summary::PageKey> {
    let laid = laid_page(layout, page)?;
    let items = item_indices(laid)
        .iter()
        .filter_map(|i| layout.flow.items.get(*i).cloned())
        .collect();
    Some(crate::summary::PageKey {
        page: Arc::clone(laid),
        items,
    })
}

/// Whether two lists of flow items are the very same items.
pub(crate) fn same_items(a: &[FlowItem], b: &[FlowItem]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|pair| match pair {
            (FlowItem::Para(x), FlowItem::Para(y)) => Arc::ptr_eq(x, y),
            (FlowItem::Table(x), FlowItem::Table(y)) => Arc::ptr_eq(x, y),
            (FlowItem::Stop(x), FlowItem::Stop(y)) => Arc::ptr_eq(x, y),
            _ => false,
        })
}
