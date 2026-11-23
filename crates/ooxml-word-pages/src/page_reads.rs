//! Owns: what a check read of each page ([`ReadMemo`]), kept across a generation's
//! versions by what the read depends on: the page as laid out (its `LaidPage` and the flow
//! items it draws, by identity), its context ([`crate::page_context`]: its number,
//! counts, section, header and footer texts, line numbers), the stories' epoch and the
//! model's context it was painted in, and the paths of the blocks it draws (a read names
//! its lines by path). A page another version lays out the same, at the same places, is
//! read as it was: a check after a local edit paints only the pages it changed.
//!
//! It is held within the engine's paint budget (the session's bounded account), the least
//! recently used let go first; what it keeps of a page is never the page (weak references
//! tell it again), and a page no version holds is swept with the memos.
//!
//! Must not: paint, keep a laid page or flow item alive, or take a read whose page differs
//! in anything the read reads.

use crate::digest_cache::{PageKey, PageRef};
use crate::page_read::PageRead;
use ooxml_base::Digest;
use ooxml_derive::{CacheBudget, Memo, Weigh};
use ooxml_word_layout::flow::FlowItem;
use ooxml_word_layout::DocumentLayout;
use ooxml_word_model::model::Block;
use ooxml_word_model::value::Text;
use std::sync::Arc;

/// A page's key: its laid page by address, its context, the stories' epoch and the
/// model's context it was painted in.
type ReadKey = (usize, Digest, u64, u64);

/// What a page read depends on beyond its key's address: the page as laid out and the
/// paths of the blocks it draws.
pub(crate) struct ReadOf<'k> {
    pub(crate) page: &'k PageKey,
    pub(crate) context: Digest,
    pub(crate) epoch: u64,
    pub(crate) model: u64,
    pub(crate) places: Arc<[Text]>,
}

impl ReadOf<'_> {
    fn key(&self) -> ReadKey {
        (
            Arc::as_ptr(&self.page.page) as usize,
            self.context,
            self.epoch,
            self.model,
        )
    }
}

/// One page as read.
#[derive(Clone)]
struct Kept {
    page: PageRef,
    places: Arc<[Text]>,
    read: Arc<PageRead>,
}

impl Weigh for Kept {
    fn weight(&self) -> usize {
        let text = |l: &crate::line_text::LineText| {
            std::mem::size_of_val(l)
                + l.path.len()
                + l.story.as_ref().map_or(0, |(_, part)| part.len())
                + l.sizes.len() * std::mem::size_of::<(i64, usize)>()
        };
        let boxes = &self.read.boxes;
        let boxed = boxes.pictures.iter().chain(&boxes.floats).map(|b| 64 + b.path.len());
        let rules = boxes.rules.iter().map(|r| 48 + r.path.len());
        std::mem::size_of::<Kept>()
            + self
                .read
                .texts
                .iter()
                .chain(&self.read.box_texts)
                .map(text)
                .sum::<usize>()
            + boxed.chain(rules).sum::<usize>()
            + self.places.iter().map(|p| 16 + p.len()).sum::<usize>()
            + self.page.weight()
    }
}

/// Each page's read, by what it depends on.
pub(crate) struct ReadMemo {
    memo: Memo<ReadKey, Kept>,
}

impl std::fmt::Debug for ReadMemo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadMemo").field("stats", &self.memo.stats()).finish()
    }
}

impl ReadMemo {
    /// An empty memo counted on `budget`.
    pub(crate) fn new(budget: Arc<dyn CacheBudget>) -> ReadMemo {
        ReadMemo {
            memo: Memo::new(budget),
        }
    }

    /// The read of the page `of` names, where a version before read it the same.
    pub(crate) fn get(&mut self, of: &ReadOf<'_>) -> Option<Arc<PageRead>> {
        let found = self.memo.get(&of.key())?;
        (found.page.is(of.page) && found.places == of.places).then_some(found.read)
    }

    /// Keep `read` as the page `of` names.
    pub(crate) fn insert(&mut self, of: &ReadOf<'_>, read: Arc<PageRead>) {
        let kept = Kept {
            page: PageRef::of(of.page),
            places: Arc::clone(&of.places),
            read,
        };
        self.memo.insert(of.key(), kept);
    }

    /// Drop the reads of pages no version holds that nothing asked for since the last
    /// sweep: how many went.
    pub(crate) fn sweep(&mut self) -> usize {
        self.memo.sweep(|kept| kept.page.alive())
    }
}

/// The paths of the blocks a page's items draw, in order: a paragraph's (and its drop
/// cap's), a table's. A read names its lines by these; an item kept from the version
/// before whose block moved names it otherwise.
pub(crate) fn places(layout: &DocumentLayout, items: &[FlowItem]) -> Arc<[Text]> {
    let paragraphs = layout.body_paragraphs();
    let body = &layout.document.body;
    let body_path = |k: usize| match body.get(k) {
        Some(Block::Paragraph(p)) => Some(p.path.clone()),
        Some(Block::Table(t)) => Some(t.path.clone()),
        None => None,
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item {
            FlowItem::Para(p) => {
                out.extend(paragraphs.get(p.block).map(|q| q.path.clone()));
                out.extend(p.drop_cap.as_ref().and_then(|d| body_path(d.body)));
            }
            FlowItem::Table(t) => out.extend(body_path(t.body)),
            FlowItem::Stop(_) => {}
        }
    }
    out.into()
}
