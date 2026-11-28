//! Owns: a Word document's changes feed (phase 3 changes-feed design): [`ChangesFeed::changes`],
//! what changed after a `since` -- the parts and blocks, and with `pages` each changed page's
//! groups (sent whole where their content changed, by offset where they only moved, by id
//! where removed), its order where it changed and the page list where it changed -- each
//! version named by the store's seq, a `since` written `<session nonce>-<seq>`
//! ([`Changes::token`]).
//!
//! The feed keeps no history: per page and group the seq it last changed at, tombstones with
//! a floor ([`records`]), and each page's SVG, latest only, in an evictable cache on the
//! document's memory budget ([`cache`]). Its published state is read without a lock; one
//! caller at a time advances it to the head, taking the page engine's lock per page it
//! paints, so writers' reflows go on between; a reader that comes while it advances answers
//! from the published state, `behind` the head. Writers never touch it. Undo and redo are
//! versions like any other: only their pages are painted.
//!
//! The pages changed are those the engine's comparison says differ, those drawing a block
//! the version's change touched, and those past the last before.
//!
//! Must not: lay a version out but through the host's engine, keep a version's layout but
//! the one it last wrote (what the next comparison reads), or answer below its floor.

mod cache;
mod records;
mod wire;

pub use records::BlockChanges;
pub(crate) use wire::hosted;

use crate::live_view::{LiveViews, ViewError};
use crate::word::Word;
use arc_swap::ArcSwap;
use cache::SvgCache;
use ooxml_base::{CacheBudget, Context};
use ooxml_session::{CallCancel, CallContext, DocId, Session};
use ooxml_store::Snapshot;
use ooxml_tools::ToolError;
use ooxml_tools_word::{laid_to_draw, WordLayout};
use ooxml_word_edit::WordDoc;
use ooxml_word_layout::paint::feed::{FeedFrame, FeedPage};
use ooxml_word_layout::paint::svg::FeedPicture;
use ooxml_word_model::ids::{StoryDelta, StoryIx};
use ooxml_word_pages::LaidVersion;
use records::{DocRecords, DocStep, PageRecords, PageSince};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

/// What a refusal names as not drawing the document.
const WHO: &str = "the changes feed";

/// One group sent whole: its id, its offset and its content in its own coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupSvg {
    /// Its id.
    pub id: String,
    /// Its offset (`translate(0,dy)`).
    pub dy: i64,
    /// Its content.
    pub svg: String,
}

/// What changed on one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageChange {
    /// The 1-based page.
    pub page: usize,
    /// Its groups' ids in drawing order, where that changed.
    pub order: Option<Vec<String>>,
    /// The groups whose content changed or that came.
    pub groups: Vec<GroupSvg>,
    /// The groups only moved: id and offset.
    pub moved: Vec<(String, i64)>,
    /// The groups removed.
    pub removed: Vec<String>,
    /// The resources of the groups sent whole, by group id (none where a group has none).
    pub defs: Vec<(String, String)>,
    /// The pictures the groups sent whole draw that were not drawn at `since`.
    pub pictures: Vec<FeedPicture>,
}

/// The pages of a [`Changes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageChanges {
    /// Every page's frame, in order, where the list (its count or a page's size) changed.
    pub list: Option<Vec<FeedFrame>>,
    /// The pages that changed, in order.
    pub changed: Vec<PageChange>,
}

/// What [`ChangesFeed::changes`] answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    /// The version answered: the store's seq.
    pub version: u64,
    /// Its state number.
    pub state: u64,
    /// The session's nonce: a `since` of another session's is stale.
    pub session: String,
    /// What the next call's `since` is: `<session>-<version>`.
    pub token: String,
    /// The `since` given.
    pub since: Option<String>,
    /// Everything is sent: no `since`, or one the feed cannot answer exactly (another
    /// session's, below its floor, past the head).
    pub full: bool,
    /// The head's seq, where the answer is of an earlier version (the feed was being
    /// advanced by another caller).
    pub behind: Option<u64>,
    /// The parts changed after `since` (every part, where full).
    pub parts: Vec<String>,
    /// The blocks changed after `since` (none listed where full).
    pub blocks: BlockChanges,
    /// The pages, where asked.
    pub pages: Option<PageChanges>,
}

/// A session's changes feed of its Word documents.
pub trait ChangesFeed {
    /// What changed in the Word document `doc` after `since` (a [`Changes::token`]; `None`:
    /// everything); with `pages`, its pages too, laid out by `layout` (the host's engine for
    /// the document) within `context`'s limit on laying out. Without `pages` nothing is laid
    /// out.
    ///
    /// # Errors
    /// [`ViewError::Session`] for a document not open, [`ViewError::NotWord`],
    /// [`ViewError::NotDrawn`] where the version is not laid out.
    fn changes(
        &self,
        doc: &DocId,
        since: Option<&str>,
        pages: bool,
        layout: &dyn WordLayout,
        context: &CallContext,
    ) -> Result<Changes, ViewError>;
}

/// What the feed published: its records of the parts and blocks, and of the pages with the
/// layout they were written from.
#[derive(Default)]
struct Published {
    doc: Option<Arc<DocAt>>,
    pages: Option<Arc<PagesAt>>,
}

/// The parts' and blocks' records, and the version they are of.
struct DocAt {
    records: DocRecords,
    at: Snapshot<WordDoc>,
}

/// The pages' records, the version and layout they were written from, and each picture's
/// first seq.
struct PagesAt {
    records: PageRecords,
    /// The version's state number.
    state: u64,
    at: Snapshot<WordDoc>,
    laid: Arc<LaidVersion>,
    pictures: Arc<HashMap<String, u64>>,
}

/// A document's changes feed.
pub(crate) struct Feed {
    budget: Arc<dyn CacheBudget>,
    published: ArcSwap<Published>,
    /// Held by the one caller advancing the feed.
    advancing: Mutex<()>,
    /// What the published records reserved of the budget.
    reserved: Mutex<usize>,
    cache: SvgCache,
}

impl Feed {
    /// A feed that has written nothing yet, within `budget`.
    pub(crate) fn new(budget: Arc<dyn CacheBudget>) -> Feed {
        Feed {
            cache: SvgCache::new(Arc::clone(&budget)),
            budget,
            published: ArcSwap::from_pointee(Published::default()),
            advancing: Mutex::new(()),
            reserved: Mutex::new(0),
        }
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        let reserved = *self.reserved.get_mut().unwrap_or_else(PoisonError::into_inner);
        self.budget.release(reserved);
    }
}

/// `since` read: the seq of a token of this session's, `None` for another's or none.
fn since_seq(since: &str, nonce: &str) -> Option<u64> {
    let (session, seq) = since.rsplit_once('-')?;
    (session == nonce).then(|| seq.parse().ok()).flatten()
}

impl ChangesFeed for Session {
    fn changes(
        &self,
        doc: &DocId,
        since: Option<&str>,
        pages: bool,
        layout: &dyn WordLayout,
        context: &CallContext,
    ) -> Result<Changes, ViewError> {
        let held = self.format_doc(doc)?;
        let kind = self.kind_of(doc)?;
        let word = Word::of(&*held).ok_or_else(|| ViewError::NotWord {
            doc: doc.to_string(),
            kind,
        })?;
        let ask = Ask {
            word,
            doc,
            layout: Some(layout),
            nonce: self.nonce(),
            ports: self.context(),
            context,
        };
        asked(&ask, since, pages)
    }
}

/// What changed after `since` (with `pages`, the pages too), as `ask` reads it.
pub(crate) fn asked(ask: &Ask<'_>, since: Option<&str>, pages: bool) -> Result<Changes, ViewError> {
    {
        let word = ask.word;
        let seq = since.and_then(|s| since_seq(s, ask.nonce));
        let (published, behind) = word.feed.current(ask, pages)?;
        let mut answer = word.feed.answer(ask, &published, seq, pages)?;
        if answer.is_none() {
            // a page's SVG could not be written again (its layout no longer painted): the
            // pages are written afresh, and everything is sent
            let published = word.feed.afresh(ask)?;
            answer = word.feed.answer(ask, &published, None, pages)?;
        }
        let mut changes = answer.ok_or_else(|| ViewError::NotDrawn(not_drawn()))?;
        changes.since = since.map(str::to_owned);
        changes.behind = behind;
        Ok(changes)
    }
}

/// One call's document and what it reads it with.
pub(crate) struct Ask<'a> {
    pub(crate) word: &'a Word,
    pub(crate) doc: &'a DocId,
    /// The engine its pages are laid out with (`None`: a call without pages, which lays
    /// nothing out).
    pub(crate) layout: Option<&'a dyn WordLayout>,
    /// The session's nonce.
    pub(crate) nonce: &'a str,
    /// The session's ports (its cancellation and clock).
    pub(crate) ports: &'a Context,
    /// The call's context.
    pub(crate) context: &'a CallContext,
}

fn not_drawn() -> ToolError {
    ToolError::internal("the pages could not be written", "changes.pages")
}

fn lock(layout: &dyn WordLayout) -> std::sync::MutexGuard<'_, ooxml_word_pages::PageEngine> {
    ooxml_word_pages::lock_engine(layout.engine())
}

impl Feed {
    /// The published state at the head (advanced to it here where it is not, `pages`
    /// included where asked), or -- where another caller is advancing it -- as published,
    /// with the head's seq it is behind.
    fn current(&self, ask: &Ask<'_>, pages: bool) -> Result<(Arc<Published>, Option<u64>), ViewError> {
        let head = ask.word.head();
        let seq = head.seq().0;
        let at_head = |p: &Published| {
            p.doc.as_ref().is_some_and(|d| d.records.seq == seq)
                && (!pages || p.pages.as_ref().is_some_and(|q| q.records.seq == seq))
        };
        let published = self.published.load_full();
        if at_head(&published) {
            return Ok((published, None));
        }
        // an answer of the published state is of one version: with pages, the parts' and
        // blocks' records of the pages' own
        let ready = |p: &Published| match (&p.doc, &p.pages) {
            (Some(_), _) if !pages => true,
            (Some(d), Some(q)) => d.records.seq == q.records.seq,
            _ => false,
        };
        let guard = match self.advancing.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) if ready(&published) => return Ok((published, Some(seq))),
            Err(_) => self.advancing.lock().unwrap_or_else(PoisonError::into_inner),
        };
        let head = ask.word.head();
        let published = self.published.load_full();
        let doc = match &published.doc {
            Some(d) if d.records.seq == head.seq().0 => Arc::clone(d),
            Some(d) => Arc::new(DocAt {
                records: d
                    .records
                    .advanced(head.seq().0, head.number(), &doc_step(d.at.doc(), head.doc())),
                at: Arc::clone(&head),
            }),
            None => Arc::new(DocAt {
                records: DocRecords::first(head.seq().0, head.number()),
                at: Arc::clone(&head),
            }),
        };
        let pages_at = match (&published.pages, pages) {
            (Some(p), _) if p.records.seq == head.seq().0 => Some(Arc::clone(p)),
            (previous, true) => Some(self.pages_advanced(ask, &head, previous.as_deref())?),
            (previous, false) => previous.clone(),
        };
        let made = Arc::new(Published {
            doc: Some(doc),
            pages: pages_at,
        });
        // answered from what was made, even where its records had no room to be kept
        let kept = self.reserve(Arc::clone(&made), &ask.word.views);
        self.published.store(kept);
        drop(guard);
        Ok((made, None))
    }

    /// The pages written afresh at the head: the records begin again (their floor is the
    /// head's seq).
    fn afresh(&self, ask: &Ask<'_>) -> Result<Arc<Published>, ViewError> {
        let _guard = self.advancing.lock().unwrap_or_else(PoisonError::into_inner);
        let head = ask.word.head();
        let pages = self.pages_advanced(ask, &head, None)?;
        let doc = Arc::new(DocAt {
            records: DocRecords::first(head.seq().0, head.number()),
            at: Arc::clone(&head),
        });
        let made = Arc::new(Published {
            doc: Some(doc),
            pages: Some(pages),
        });
        let kept = self.reserve(Arc::clone(&made), &ask.word.views);
        self.published.store(kept);
        Ok(made)
    }

    /// `made`, its records reserved from the budget: room made in the eviction order (older
    /// views' layouts, the least recently sent pages' SVG, then the tombstones -- the floor
    /// raised); where there is still none, its pages' records are let go (the next call with
    /// pages writes them afresh).
    fn reserve(&self, made: Arc<Published>, views: &LiveViews) -> Arc<Published> {
        let mut reserved = self.reserved.lock().unwrap_or_else(PoisonError::into_inner);
        self.budget.release(*reserved);
        *reserved = 0;
        let mut made = made;
        loop {
            let weight = weight_of(&made);
            if self.budget.reserve(weight) {
                *reserved = weight;
                return made;
            }
            if views.let_go_older() || self.cache.evict_one() {
                continue;
            }
            match made.pages.as_ref().and_then(|p| p.records.pruned().map(|r| (p, r))) {
                Some((p, records)) => {
                    made = Arc::new(Published {
                        doc: made.doc.clone(),
                        pages: Some(Arc::new(PagesAt {
                            records,
                            state: p.state,
                            at: Arc::clone(&p.at),
                            laid: Arc::clone(&p.laid),
                            pictures: Arc::clone(&p.pictures),
                        })),
                    });
                }
                None if made.pages.is_some() => {
                    made = Arc::new(Published {
                        doc: made.doc.clone(),
                        pages: None,
                    });
                }
                None => return made,
            }
        }
    }
}

/// What the published records weigh on the budget.
fn weight_of(published: &Published) -> usize {
    published.doc.as_ref().map_or(0, |d| d.records.weight())
        + published.pages.as_ref().map_or(0, |p| {
            p.records.weight() + p.pictures.keys().map(|h| h.len() + 24).sum::<usize>()
        })
}

/// The change from `before` to `now` as the records take it: the parts that differ, and the
/// blocks changed, come and gone by id (a story changed whole is in its part only; a block
/// changed only by stamping is not changed).
fn doc_step(before: &WordDoc, now: &WordDoc) -> DocStep {
    let delta = now.delta_from(before);
    let mut step = DocStep {
        parts: delta.changed_parts().into_iter().map(str::to_owned).collect(),
        ..DocStep::default()
    };
    let (Ok(old), Ok(new)) = (before.docx(), now.docx()) else {
        return step;
    };
    let part_of = |docx: &ooxml_word_model::docx::Docx, name: &str| docx.parts.iter().find(|p| p.part == name).cloned();
    for story in &delta.stories {
        let StoryDelta::Blocks(blocks) = &story.delta else {
            continue;
        };
        let suffix = if story.story == StoryIx::BODY {
            String::new()
        } else {
            format!("@{}", story.part)
        };
        let id = |docx: &ooxml_word_model::docx::Docx, key| {
            part_of(docx, &story.part)
                .and_then(|p| p.ids.id_of_key(key))
                .map(|id| format!("{id}{suffix}"))
        };
        step.changed.extend(blocks.changed.iter().filter_map(|k| id(&new, *k)));
        step.came.extend(blocks.came.iter().filter_map(|(k, _)| id(&new, *k)));
        step.gone.extend(blocks.gone.iter().filter_map(|(k, _)| id(&old, *k)));
    }
    step
}

impl Feed {
    /// The pages' records advanced to `head` from `previous` (written afresh where there is
    /// none): laid out by the host's engine within the call's limit; the pages the engine's
    /// comparison says differ, those drawing a block the change touched and those past the
    /// last before written again -- the engine's lock taken per page -- and kept in the
    /// cache.
    fn pages_advanced(
        &self,
        ask: &Ask<'_>,
        head: &Snapshot<WordDoc>,
        previous: Option<&PagesAt>,
    ) -> Result<Arc<PagesAt>, ViewError> {
        let cancel = CallCancel::new(ask.context, &*ask.ports.cancel, &*ask.ports.clock);
        let laying = cancel.layout();
        let limit = laying.as_ref().map(crate::word::limit);
        let package = (head.doc().package(), &*ask.doc.to_string());
        let layout = ask.layout.ok_or_else(|| ViewError::NotDrawn(not_drawn()))?;
        let laid = laid_to_draw(WHO, layout, limit, package, &cancel).map_err(ViewError::NotDrawn)?;
        // the next advance compares with it: digested before the engine stops painting it
        lock(layout).watch(&laid);
        let seq = head.seq().0;
        let count = laid.page_count();
        let to_write: BTreeSet<usize> = match previous {
            Some(p) => changed_pages(layout, p, &laid, head),
            None => (0..count).collect(),
        };
        let mut written = BTreeMap::new();
        for page in to_write.into_iter().filter(|p| *p < count) {
            let made = lock(layout).feed_page(&laid, page).map(|(made, _)| made);
            let made = made.ok_or_else(|| ViewError::NotDrawn(not_drawn()))?;
            written.insert(page, made);
        }
        let mut pictures = previous.map_or_else(HashMap::new, |p| (*p.pictures).clone());
        for made in written.values() {
            for picture in made.groups.iter().flat_map(|g| &g.pictures) {
                pictures.entry(picture.hash.clone()).or_insert(seq);
            }
        }
        let records = match previous {
            Some(p) => p.records.advanced(seq, count, &written),
            None => PageRecords::first(&written.values().cloned().collect::<Vec<_>>(), seq),
        };
        self.cache.truncate(count);
        for (page, made) in written {
            self.cache
                .keep(page, seq, &Arc::new(made), &|| ask.word.views.let_go_older());
        }
        Ok(Arc::new(PagesAt {
            records,
            state: head.number(),
            at: Arc::clone(head),
            laid,
            pictures: Arc::new(pictures),
        }))
    }

    /// What changed after `since` (everything where `None`, or where it is not answered
    /// exactly) in `published`; `None` where a page's SVG was let go and can no longer be
    /// written again.
    fn answer(
        &self,
        ask: &Ask<'_>,
        published: &Published,
        since: Option<u64>,
        pages: bool,
    ) -> Result<Option<Changes>, ViewError> {
        let doc = published.doc.as_ref().ok_or_else(|| ViewError::NotDrawn(not_drawn()))?;
        let nonce = ask.nonce;
        let (version, state) = match (pages, &published.pages) {
            (true, Some(p)) => (p.records.seq, p.state),
            _ => (doc.records.seq, doc.records.state),
        };
        let exact = since.filter(|s| {
            doc.records.answers(*s) && (!pages || published.pages.as_ref().is_some_and(|p| p.records.answers(*s)))
        });
        let (parts, blocks) = match exact {
            Some(s) => (doc.records.parts_since(s), doc.records.blocks_since(s)),
            None => (
                doc.at.doc().package().part_names().into_iter().collect(),
                BlockChanges::default(),
            ),
        };
        let pages = match (pages, &published.pages) {
            (true, Some(p)) => match self.page_changes(ask, p, exact) {
                Some(found) => Some(found),
                None => return Ok(None),
            },
            _ => None,
        };
        Ok(Some(Changes {
            version,
            state,
            session: nonce.to_owned(),
            token: format!("{nonce}-{version}"),
            since: None,
            full: exact.is_none(),
            behind: None,
            parts,
            blocks,
            pages,
        }))
    }

    /// The pages changed after `since` (every page where `None`), their groups' SVG from the
    /// cache or written again; `None` where one can no longer be written.
    fn page_changes(&self, ask: &Ask<'_>, at: &PagesAt, since: Option<u64>) -> Option<PageChanges> {
        let records = &at.records;
        let list = since
            .is_none_or(|s| records.list_since(s))
            .then(|| records.pages.iter().map(|p| p.frame.clone()).collect());
        let mut changed = Vec::new();
        for found in records.since(since) {
            let record = records.pages.get(found.page)?;
            let written = match self.cache.get(found.page, record.changed..=records.seq) {
                Some(page) => page,
                None => {
                    let made = Arc::new(lock(ask.layout?).feed_page(&at.laid, found.page)?.0);
                    self.cache
                        .keep(found.page, records.seq, &made, &|| ask.word.views.let_go_older());
                    made
                }
            };
            changed.push(page_change(&found, &written, &at.pictures, since));
        }
        Some(PageChanges { list, changed })
    }
}

/// One page's change: the groups `found` names, from `written`.
fn page_change(
    found: &PageSince,
    written: &FeedPage,
    pictures: &HashMap<String, u64>,
    since: Option<u64>,
) -> PageChange {
    let wanted: HashSet<&str> = found.groups.iter().map(String::as_str).collect();
    let mut change = PageChange {
        page: found.page + 1,
        order: found.order.clone(),
        groups: Vec::new(),
        moved: found.moved.clone(),
        removed: found.removed.clone(),
        defs: Vec::new(),
        pictures: Vec::new(),
    };
    let mut seen = HashSet::new();
    for g in written.groups.iter().filter(|g| wanted.contains(g.id.as_str())) {
        change.groups.push(GroupSvg {
            id: g.id.clone(),
            dy: g.dy,
            svg: g.content.clone(),
        });
        if !g.defs.is_empty() {
            change.defs.push((g.id.clone(), g.defs.clone()));
        }
        for picture in &g.pictures {
            let new = since.is_none_or(|s| pictures.get(&picture.hash).is_none_or(|at| *at > s));
            if new && seen.insert(picture.hash.clone()) {
                change.pictures.push(picture.clone());
            }
        }
    }
    change
}

/// The pages of `laid` to write again after `previous`: those the engine's comparison says
/// differ (every page where it cannot compare), those whose groups `previous` wrote draw a
/// block the change from `previous`'s version touched or the top-level block holding it (a
/// text box's text, a drawing's paint: what a page's comparison does not digest; a page that
/// draws it anew is one the comparison lists), and those past the last before.
fn changed_pages(
    layout: &dyn WordLayout,
    previous: &PagesAt,
    laid: &Arc<LaidVersion>,
    head: &Snapshot<WordDoc>,
) -> BTreeSet<usize> {
    let count = laid.page_count();
    if Arc::ptr_eq(&previous.laid, laid) {
        return BTreeSet::new();
    }
    let compared = lock(layout).compare(laid, &previous.laid, &HashMap::new());
    let Some(reflow) = compared else {
        return (0..count).collect();
    };
    let mut out: BTreeSet<usize> = reflow.changed.iter().map(|p| p - 1).collect();
    out.extend(previous.records.pages.len()..count);
    let touched = touched_blocks(previous.at.doc(), head.doc());
    if !touched.is_empty() {
        for (page, record) in previous.records.pages.iter().enumerate().take(count) {
            if record.groups.iter().any(|g| touched.contains(group_block(&g.id))) {
                out.insert(page);
            }
        }
    }
    out
}

/// The block a body group of the feed's is drawn for, by id: a paragraph's (`p:<id>`), a
/// drawing's paragraph's (`d:<id>/<run>.<index>`), a table row's table's (`<table
/// id>/<row id>/under`).
fn group_block(id: &str) -> &str {
    let id = id.strip_prefix("d:").unwrap_or(id);
    id.split('/').next().unwrap_or(id)
}

/// The body's blocks (by id) the change from `before` to `now` changed or brought, and the
/// top-level blocks holding them.
fn touched_blocks(before: &WordDoc, now: &WordDoc) -> HashSet<String> {
    let delta = now.delta_from(before);
    let Ok(docx) = now.docx() else {
        return HashSet::new();
    };
    let Some(body) = docx.parts.iter().find(|p| p.part == docx.main) else {
        return HashSet::new();
    };
    let mut out = HashSet::new();
    for story in delta.stories.iter().filter(|s| s.story == StoryIx::BODY) {
        let StoryDelta::Blocks(blocks) = &story.delta else {
            continue;
        };
        for key in blocks.changed.iter().chain(blocks.came.iter().map(|(k, _)| k)) {
            out.extend(body.ids.id_of_key(*key).map(|id| id.to_string()));
            out.extend(top_block(body, *key));
        }
    }
    out
}

/// The id of the body's top-level block holding the block `key` names.
fn top_block(body: &ooxml_word_model::docx::StoryPart, key: ooxml_word_model::ids::BlockKey) -> Option<String> {
    let place = body.ids.place_of(key)?;
    let depth = body.ids.container().len() + 1;
    let top = body.ids.key_at(StoryIx::BODY, place.get(..depth)?)?;
    body.ids.id_of_key(top).map(|id| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::since_seq;

    #[test]
    fn a_token_names_its_session_and_seq() {
        assert_eq!(since_seq("0123456789abcdef-42", "0123456789abcdef"), Some(42));
        assert_eq!(since_seq("fedcba9876543210-42", "0123456789abcdef"), None);
        assert_eq!(since_seq("0123456789abcdef-x", "0123456789abcdef"), None);
        assert_eq!(since_seq("42", "0123456789abcdef"), None);
    }
}
