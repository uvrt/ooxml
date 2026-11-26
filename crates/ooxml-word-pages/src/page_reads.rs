//! Owns: the page tier of the caching policy ([`ReadMemo`]; ADR-008): each page's entry
//! ([`crate::page_entry::PageEntry`]: what a check read of it, and its facts'
//! contributions made from that read), kept across a generation's versions by what the
//! read depends on: the page as laid out (its `LaidPage` and the flow
//! items it draws, by identity), its context ([`crate::page_context`]: its number,
//! counts, section, header and footer texts, line numbers), the stories' epoch and the
//! model's context it was painted in, and the paths and keys of the blocks it draws (a read
//! names what it read by key: [`crate::line_text::ItemPlace`]; [`PageBlocks`]). A page another version
//! lays out the same, of the same blocks at the same places, is read as it was: a check
//! after a local edit paints only the pages it changed. The keys are checked, never part of
//! the memo's key (ADR-008): an edit that stamps a block keeps its content and its place,
//! and renames it.
//!
//! A read is kept by its page's body context; its stories' context (the counts, the
//! `STYLEREF` texts) beside it. A page whose stories show other values -- another page
//! count after an edit added a page -- keeps its body's part of the read and has only its
//! stories painted again ([`Found::Body`]), where its read was split into the two parts
//! when made ([`Split`]: the stories' paint is the read's tail).
//!
//! It is held within the engine's paint budget (the session's bounded account), the least
//! recently used let go first; what it keeps of a page is never the page (weak references
//! tell it again), and a page no version holds is swept with the memos.
//!
//! Must not: paint, keep a laid page or flow item alive, or take a read whose page differs
//! in anything the read reads.

use crate::digest_cache::{PageKey, PageRef};
use crate::page_context::PageContext;
use crate::page_entry::PageEntry;
use crate::page_read::PageRead;
use ooxml_base::Digest;
use ooxml_derive::{CacheBudget, Memo, Weigh};
use ooxml_word_layout::flow::FlowItem;
use ooxml_word_layout::DocumentLayout;
use ooxml_word_model::ids::BlockKey;
use ooxml_word_model::model::{Block, Table};
use ooxml_word_model::value::Text;
use std::sync::{Arc, Weak};

/// A page's key: its laid page by address, its context, the stories' epoch and the
/// model's context it was painted in.
type ReadKey = (usize, Digest, u64, u64);

/// What a page read depends on beyond its key's address: the page as laid out, and the
/// paths and keys of the blocks it draws.
pub(crate) struct ReadOf<'k> {
    pub(crate) page: &'k PageKey,
    pub(crate) context: PageContext,
    pub(crate) epoch: u64,
    pub(crate) model: u64,
    pub(crate) places: Arc<[Text]>,
    pub(crate) blocks: PageBlocks,
}

impl ReadOf<'_> {
    fn key(&self) -> ReadKey {
        (
            Arc::as_ptr(&self.page.page) as usize,
            self.context.body,
            self.epoch,
            self.model,
        )
    }
}

/// How much of a read is its body's: its stories' paint is the rest (their lines, text
/// boxes' lines and rules; a story draws none of the body's pictures or drawings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Split {
    texts: usize,
    box_texts: usize,
    rules: usize,
}

impl Split {
    /// Where `read` (a whole page's) ends in `stories` (its stories' alone), if it does.
    pub(crate) fn of(read: &PageRead, stories: &PageRead) -> Option<Split> {
        let tail = |all: usize, part: usize| all.checked_sub(part);
        let split = Split {
            texts: tail(read.texts.len(), stories.texts.len())?,
            box_texts: tail(read.box_texts.len(), stories.box_texts.len())?,
            rules: tail(read.boxes.rules.len(), stories.boxes.rules.len())?,
        };
        let ends = read.texts.get(split.texts..)? == stories.texts.as_slice()
            && read.box_texts.get(split.box_texts..)? == stories.box_texts.as_slice()
            && read.boxes.rules.get(split.rules..)? == stories.boxes.rules.as_slice()
            && stories.boxes.pictures.is_empty()
            && stories.boxes.floats.is_empty();
        ends.then_some(split)
    }

    /// `read`'s body part followed by `stories`.
    pub(crate) fn joined(self, read: &PageRead, stories: PageRead) -> PageRead {
        let head = |all: &[crate::line_text::LineText], k: usize| all.get(..k).unwrap_or_default().to_vec();
        let mut out = PageRead {
            texts: head(&read.texts, self.texts),
            box_texts: head(&read.box_texts, self.box_texts),
            boxes: read.boxes.clone(),
        };
        out.boxes.rules.truncate(self.rules);
        out.texts.extend(stories.texts);
        out.box_texts.extend(stories.box_texts);
        out.boxes.rules.extend(stories.boxes.rules);
        out
    }
}

/// One page as read.
#[derive(Clone)]
struct Kept {
    page: PageRef,
    places: Arc<[Text]>,
    blocks: PageBlocks,
    /// Its stories' context.
    stories: Digest,
    split: Option<Split>,
    read: Arc<PageEntry>,
}

/// A page's read as kept.
pub(crate) enum Found {
    /// The page's entry: its read and its facts' contributions.
    Whole(Arc<PageEntry>),
    /// An entry whose read's body part is the page's: its stories are to be painted again.
    Body(Arc<PageEntry>, Split),
}

impl Weigh for Kept {
    fn weight(&self) -> usize {
        let place = |p: &crate::line_text::ItemPlace| match p {
            crate::line_text::ItemPlace::Anchored(_) => 0,
            crate::line_text::ItemPlace::Below(_, below) => below.len(),
            crate::line_text::ItemPlace::Path(path) => path.len(),
        };
        let text = |l: &crate::line_text::LineText| {
            std::mem::size_of_val(l)
                + place(&l.place)
                + l.at.cell.as_ref().map_or(0, |(_, below)| below.len())
                + l.story.as_ref().map_or(0, |(_, part)| part.len())
                + l.sizes.len() * std::mem::size_of::<(i64, usize)>()
        };
        let read = self.read.read();
        let boxes = &read.boxes;
        let boxed = boxes.pictures.iter().chain(&boxes.floats).map(|b| 64 + place(&b.place));
        let rules = boxes.rules.iter().map(std::mem::size_of_val);
        std::mem::size_of::<Kept>()
            + read.texts.iter().chain(&read.box_texts).map(text).sum::<usize>()
            + boxed.chain(rules).sum::<usize>()
            + self.places.iter().map(|p| 16 + p.len()).sum::<usize>()
            + self.blocks.weight()
            + self.page.weight()
            + self.read.parts_weight()
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

    /// The read of the page `of` names, where a version before read it the same; or its
    /// body part, where its stories showed other values.
    pub(crate) fn get(&mut self, of: &ReadOf<'_>) -> Option<Found> {
        let found = self.memo.get(&of.key())?;
        if !(found.page.is(of.page) && found.places == of.places && found.blocks == of.blocks) {
            return None;
        }
        if found.stories == of.context.stories {
            return Some(Found::Whole(found.read));
        }
        found.split.map(|split| Found::Body(found.read, split))
    }

    /// Keep `read` as the page `of` names, split into its body's and its stories' parts
    /// where `split` says so.
    pub(crate) fn insert(&mut self, of: &ReadOf<'_>, read: Arc<PageEntry>, split: Option<Split>) {
        let kept = Kept {
            page: PageRef::of(of.page),
            places: Arc::clone(&of.places),
            blocks: of.blocks.clone(),
            stories: of.context.stories,
            split,
            read,
        };
        self.memo.insert(of.key(), kept);
    }

    /// What it holds: its entries and the bytes reserved for them.
    pub(crate) fn stats(&self) -> ooxml_derive::MemoStats {
        self.memo.stats()
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

/// What names the blocks a page's items draw, in order ([`page_blocks`]): a paragraph's
/// key, a table itself (its `Arc`, held weakly: a table and every block in it keep their
/// keys while it is the same table). What a read names its lines and boxes by: an item
/// kept from the version before whose blocks were renamed (stamped) names them otherwise.
#[derive(Debug, Clone)]
pub(crate) struct PageBlocks(Arc<[Drawn]>);

/// One block a page's item draws.
#[derive(Debug, Clone)]
enum Drawn {
    Paragraph(Option<BlockKey>),
    Table(Weak<Table>),
}

impl PartialEq for PageBlocks {
    fn eq(&self, other: &PageBlocks) -> bool {
        self.0.len() == other.0.len()
            && self.0.iter().zip(other.0.iter()).all(|pair| match pair {
                (Drawn::Paragraph(a), Drawn::Paragraph(b)) => a == b,
                (Drawn::Table(a), Drawn::Table(b)) => Weak::ptr_eq(a, b),
                _ => false,
            })
    }
}

impl PageBlocks {
    /// What it holds, bytes.
    pub(crate) fn weight(&self) -> usize {
        self.0.len() * std::mem::size_of::<Drawn>()
    }
}

/// The blocks a page's items draw ([`PageBlocks`]): a paragraph's (and its drop cap's), a
/// table's.
pub(crate) fn page_blocks(layout: &DocumentLayout, items: &[FlowItem]) -> PageBlocks {
    let paragraphs = layout.body_paragraphs();
    let body = &layout.document.body;
    let block = |block: &Block| match block {
        Block::Paragraph(p) => Drawn::Paragraph(p.key),
        Block::Table(t) => Drawn::Table(Arc::downgrade(t)),
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item {
            FlowItem::Para(p) => {
                out.push(Drawn::Paragraph(paragraphs.get(p.block).and_then(|q| q.key)));
                if let Some(cap) = &p.drop_cap {
                    out.extend(body.get(cap.body).map(block));
                }
            }
            FlowItem::Table(t) => out.extend(body.get(t.body).map(block)),
            FlowItem::Stop(_) => {}
        }
    }
    PageBlocks(out.into())
}
