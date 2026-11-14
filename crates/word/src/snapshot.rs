//! A read-only snapshot of one layout: the flow's compact arrays and their pagination, with
//! what checks ask -- the page of a block, and facts over a page range, placed lazily.

use crate::paginate::{place_page, Doc, FItem, It, Pagination, Pos};

pub struct Snapshot {
    pub doc: Doc,
    pub items: Vec<It>,
    pub pag: Pagination,
    /// Item -> its first block (ascending).
    blocks: Vec<u32>,
}

#[derive(Default, Debug, Clone, Copy)]
pub struct Facts {
    pub lines: u64,
    /// Lines (a table row's part) reaching past the bottom margin.
    pub past_bottom: u64,
    /// keepNext paragraphs whose next item starts on another page.
    pub keep_next_broken: u64,
    /// keepLines paragraphs split across pages.
    pub keep_lines_broken: u64,
}

impl Snapshot {
    pub fn new(doc: Doc, items: Vec<It>, pag: Pagination) -> Snapshot {
        let blocks = items.iter().map(|i| i.block()).collect();
        Snapshot { doc, items, pag, blocks }
    }
    pub fn pages(&self) -> usize {
        self.pag.starts.len()
    }
    /// The 0-based page (blank pages counted) a block's first line is on.
    pub fn page_of_block(&self, block: u32) -> Option<u32> {
        let k = self.blocks.partition_point(|b| *b <= block);
        if k == 0 {
            return None;
        }
        let item = (k - 1) as u32;
        let pos = Pos::at(item, 0);
        let p = self.pag.starts.partition_point(|s| *s <= pos);
        if p == 0 {
            return None;
        }
        Some(self.pag.page_at[p - 1])
    }
    /// Facts over the laid-out pages `[from, to)` (indexes into the pages laid out).
    pub fn facts(&self, from: usize, to: usize) -> Facts {
        let mut f = Facts::default();
        let to = to.min(self.pages());
        for k in from..to {
            let placed = place_page(&self.doc, &self.items, &self.pag, k);
            f.lines += placed.len() as u64;
            f.past_bottom += placed.iter().filter(|p| p.end > p.bottom).count() as u64;
            // the page's last item, against the next page's start
            if let Some(next) = self.pag.starts.get(k + 1) {
                if let Some(last) = placed.last() {
                    if let FItem::P(p) = &*self.items[last.item as usize] {
                        let ends_here = last.line as usize + 1 == p.lines.len();
                        if ends_here && p.keep_next && next.item == last.item + 1 && next.line == 0 {
                            f.keep_next_broken += 1;
                        }
                        if !ends_here && p.keep_lines && next.item == last.item {
                            f.keep_lines_broken += 1;
                        }
                    }
                }
            }
        }
        f
    }
}
