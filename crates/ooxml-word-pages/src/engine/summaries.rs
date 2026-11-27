//! Owns: a version's pages digested and compared by its engine: each page's digests
//! ([`PageEngine::page_digests`]), a version's summary ([`PageEngine::summary`]) and its
//! contents ([`PageEngine::contents`]), and the comparison of two versions page by page
//! ([`PageEngine::compare`]) -- a page both lay out the same, in the same context, is the
//! same page and is not painted; every other is digested in both, each page's digests
//! kept across versions by the page as laid out ([`crate::digest_cache`]) -- with the
//! versions a comparison may yet read ([`PageEngine::watch`]) digested whole before the
//! engine can no longer paint them (a new generation: a version of a context its generation
//! left is painted in its own, [`super::painters`]).
//!
//! Must not: answer otherwise than [`crate::summary::Summary::compare`] of the two
//! versions' whole summaries would.

use super::{PageDigests, PageEngine, KEY};
use crate::digests::Lines;
use crate::summary::{canon, first_pages, page_change, page_summary, reflow_of};
use crate::summary::{Counts, PageSummary, Reflow, Summary};
use crate::version::{LaidVersion, VersionPages};
use std::collections::HashMap;
use std::sync::{Arc, Weak};

impl PageEngine {
    /// What page `page` of `version` paints, digested: what besides its lines (its frame,
    /// rules, pictures and drawings), and each line by its paragraph's path and story --
    /// what tells two versions' pages apart without a reader reading them.
    pub fn page_digests(&mut self, version: &LaidVersion, page: usize) -> Option<PageDigests> {
        let paint = self.painted_page(version, page, false)?;
        Some(crate::digests::digests(
            &paint,
            &version.layout.document.names,
            KEY,
            Lines::Page,
        ))
    }

    /// `version`'s pages compared as the layouts themselves are (docx-agent's
    /// `DocumentLayout.compare`, over `Page.text_lines`): its summary with every line of
    /// text a page draws, its text boxes' too; made whole, never kept.
    pub fn contents(&mut self, version: &LaidVersion) -> Option<Summary> {
        let mut pages = Vec::with_capacity(version.layout.page_count());
        for page in 0..version.layout.page_count() {
            let paint = self.painted_page(version, page, false)?;
            let digests = crate::digests::digests(&paint, &version.layout.document.names, KEY, Lines::WithTextBoxes);
            pages.push(Arc::new(page_summary(digests, version)));
        }
        let known = pages.len();
        Some(Summary {
            pages,
            known,
            names: Arc::clone(&version.layout.document.names),
        })
    }

    /// Whether `version` is painted as it was laid out: of this engine's generation (in
    /// whichever of its contexts it was laid out).
    pub(super) fn paints(&self, version: &LaidVersion) -> bool {
        self.generation.as_ref().is_some_and(|g| g.number == version.generation)
    }

    /// `version`'s summary (docx-agent's `LayoutSummary`), made once: each page's taken as
    /// digested where a version before laid it out the same in the same context, the rest
    /// painted and digested; `None` where it was not made and `version` is no longer
    /// painted by this engine (another generation).
    pub fn summary(&mut self, version: &LaidVersion) -> Option<Arc<Summary>> {
        if let Some(found) = version.summary.get() {
            return Some(Arc::clone(found));
        }
        let pages = self.version_pages(version)?;
        let mut made = Vec::with_capacity(version.page_count());
        for page in 0..version.page_count() {
            made.push(self.page_summary_of(version, &pages, page)?);
        }
        let made = Arc::new(Summary {
            known: made.len(),
            pages: made,
            names: Arc::clone(&version.layout.document.names),
        });
        Some(Arc::clone(version.summary.get_or_init(|| made)))
    }

    /// What changed from `before` to `now` (two versions this engine laid out), as
    /// [`Summary::compare`] of their summaries says, ids followed through `aliases`: page
    /// by page, painting only the pages that are not the same page in both. `None` where
    /// a version's pages can no longer be digested (not painted by this engine, and no
    /// summary of it made).
    pub fn compare(
        &mut self,
        now: &LaidVersion,
        before: &LaidVersion,
        aliases: &HashMap<String, String>,
    ) -> Option<Reflow> {
        if before.summary.get().is_some() || !self.paints(before) || !self.paints(now) {
            let (before, now) = (self.summary(before)?, self.summary(now)?);
            return Some(now.compare(&before, aliases));
        }
        let (was, is) = (self.version_pages(before)?, self.version_pages(now)?);
        let counts = Counts {
            before: (before.page_count(), before.page_count()),
            now: (now.page_count(), now.page_count()),
        };
        let (ids_before, ids_now) = (body_ids(&was, aliases), body_ids(&is, aliases));
        let first_before = first_pages(ids_before.iter().map(|page| page.iter().copied()));
        let first_now = first_pages(ids_now.iter().map(|page| page.iter().copied()));
        let mut failed = false;
        let change = |number: usize| {
            let page = number - 1;
            if page >= was.ids.len() || page >= is.ids.len() {
                return None;
            }
            if same_page((before, &was), (now, &is), page) {
                self.stats.pages_digested.1 += 2;
                return Some(None);
            }
            let found = self
                .page_summary_of(before, &was, page)
                .zip(self.page_summary_of(now, &is, page));
            let Some((old, new)) = found else {
                failed = true;
                return Some(None);
            };
            Some(if Arc::ptr_eq(&old, &new) {
                None
            } else {
                page_change(&old, &new, aliases)
            })
        };
        let reflow = reflow_of(&counts, change, &first_before, &first_now);
        (!failed).then_some(reflow)
    }

    /// Keep `version` comparable: a comparison may yet read it (a check's last), so it is
    /// digested whole before this engine can no longer paint it. Held weakly: a version
    /// no one holds is not digested.
    pub fn watch(&mut self, version: &Arc<LaidVersion>) {
        self.watched.retain(|w| w.strong_count() > 0);
        if !self
            .watched
            .iter()
            .any(|w| std::ptr::eq(w.as_ptr(), Arc::as_ptr(version)))
        {
            self.watched.push(Arc::downgrade(version));
        }
    }

    /// Digest every version watched that this engine still paints and has no summary of
    /// (its generation is about to go).
    pub(super) fn settle_watched(&mut self) {
        let watched: Vec<Arc<LaidVersion>> = self.watched.iter().filter_map(Weak::upgrade).collect();
        for version in &watched {
            if version.summary.get().is_none() && self.paints(version) {
                let _ = self.summary(version);
            }
        }
        self.watched.retain(|w| w.strong_count() > 0);
    }

    /// Each page's body lines' ids (`None`: a line of a paragraph the version does not
    /// name), in drawing order, as a comparison reads them from the pages' geometry --
    /// what a summary's body lines are named, without painting; `None` where this engine
    /// no longer paints `version`.
    pub fn body_ids(&mut self, version: &LaidVersion) -> Option<Vec<Vec<Option<String>>>> {
        let pages = self.version_pages(version)?;
        Some(
            pages
                .ids
                .iter()
                .map(|page| page.iter().map(|id| id.as_deref().map(str::to_owned)).collect())
                .collect(),
        )
    }

    /// Hold at most `bytes` of page digests per generation (from now on: the current
    /// generation's are let go).
    pub fn set_digest_limit(&mut self, bytes: usize) {
        self.digest_limit = bytes;
        if let Some(generation) = self.generation.as_mut() {
            generation.digests = crate::digest_cache::DigestCache::new(bytes);
        }
    }

    /// What the generation's page digests hold: `(pages, bytes)`.
    pub fn digest_cache(&self) -> (usize, usize) {
        self.generation.as_ref().map_or((0, 0), |g| {
            let stats = g.digests.stats();
            (stats.entries, stats.bytes)
        })
    }

    /// The lines the body draws on `version`'s pages, all pages together (docx-agent's
    /// `layout_statistics`: `app.xml`'s `Lines`); `None` where this engine no longer
    /// paints it.
    pub fn body_lines(&mut self, version: &LaidVersion) -> Option<usize> {
        let pages = self.version_pages(version)?;
        Some(pages.keys.iter().map(|page| page.len()).sum())
    }

    /// What a comparison reads of `version`'s pages without painting them, made once;
    /// `None` where this engine no longer paints it.
    fn version_pages(&mut self, version: &LaidVersion) -> Option<Arc<VersionPages>> {
        if let Some(found) = version.unpainted.pages.get() {
            return Some(Arc::clone(found));
        }
        let contexts = self.page_contexts(version)?;
        let layout = &version.layout;
        let donor = self.generation.as_mut().and_then(|g| g.read_before.take());
        let names = &layout.document.names;
        let mut pages = Vec::with_capacity(layout.page_count());
        let mut blocks = Vec::with_capacity(layout.page_count());
        let mut keys = Vec::with_capacity(layout.page_count());
        for page in 0..layout.page_count() {
            let key = crate::page_said::page_key(layout, page);
            let drawn = key
                .as_ref()
                .map(|key| crate::page_reads::page_blocks(layout, &key.items));
            // A page laid out the same as the version read before's same page, of the same
            // blocks, draws the same body lines: their keys are taken, not read from its
            // geometry again.
            let taken = donor.as_ref().zip(key.as_ref()).and_then(|(donor, key)| {
                let same = donor.pages.get(page)?.as_ref()?.is(key) && donor.blocks.get(page)? == &drawn;
                same.then(|| donor.keys.get(page).cloned()).flatten()
            });
            let found = taken.unwrap_or_else(|| {
                ooxml_word_layout::paint::body_line_keys(layout, page)
                    .into_iter()
                    .filter(|key| key.is_none_or(|k| k.story() == ooxml_word_model::ids::StoryIx::BODY))
                    .collect()
            });
            pages.push(key.as_ref().map(crate::digest_cache::PageRef::of));
            blocks.push(drawn);
            keys.push(found);
        }
        // A paragraph's lines stand together: its id is asked once for them.
        let mut last: Option<(
            Option<ooxml_word_model::ids::BlockKey>,
            Option<ooxml_word_model::value::Text>,
        )> = None;
        let ids = keys
            .iter()
            .map(|page: &Arc<[Option<ooxml_word_model::ids::BlockKey>]>| {
                page.iter()
                    .map(|key| match &last {
                        Some((was, id)) if was == key => id.clone(),
                        _ => {
                            let id = key
                                .and_then(|k| names.id(k))
                                .map(|id| ooxml_word_model::value::Text::from(id.to_string().as_str()));
                            last = Some((*key, id.clone()));
                            id
                        }
                    })
                    .collect()
            })
            .collect();
        let made = Arc::new(VersionPages {
            contexts,
            pages,
            blocks,
            keys,
            ids,
        });
        let made = Arc::clone(version.unpainted.pages.get_or_init(|| made));
        if let Some(generation) = self.generation.as_mut() {
            generation.read_before = Some(Arc::clone(&made));
        }
        Some(made)
    }

    /// Page `page`'s summary of `version`: from its summary where made, else as digested
    /// where the page was laid out the same in the same context before, else painted and
    /// digested (and kept).
    fn page_summary_of(
        &mut self,
        version: &LaidVersion,
        pages: &VersionPages,
        page: usize,
    ) -> Option<Arc<PageSummary>> {
        if let Some(made) = version.summary.get() {
            return made.pages.get(page).cloned();
        }
        let key = crate::page_said::page_key(&version.layout, page);
        let context = pages.contexts.get(page).copied().unwrap_or_default().whole;
        let drawn = key.as_ref().map(|key| {
            pages
                .blocks
                .get(page)
                .cloned()
                .flatten()
                .unwrap_or_else(|| crate::page_reads::page_blocks(&version.layout, &key.items))
        });
        if let (Some(key), Some(drawn)) = (&key, &drawn) {
            let found = self
                .generation
                .as_mut()
                .and_then(|g| g.digests.get(key, drawn, context, version.epoch));
            if found.is_some() {
                self.stats.pages_digested.1 += 1;
                return found;
            }
        }
        let summary = Arc::new(page_summary(self.page_digests(version, page)?, version));
        self.stats.pages_digested.0 += 1;
        if let (Some(key), Some(drawn), Some(generation)) = (&key, drawn, self.generation.as_mut()) {
            generation
                .digests
                .insert(key, drawn, context, version.epoch, Arc::clone(&summary));
        }
        Some(summary)
    }
}

/// Each page's body ids, followed through `aliases`.
fn body_ids<'a>(pages: &'a VersionPages, aliases: &'a HashMap<String, String>) -> Vec<Vec<&'a str>> {
    pages
        .ids
        .iter()
        .map(|page| page.iter().flatten().map(|id| canon(aliases, id)).collect())
        .collect()
}

/// Whether page `page` is the same page in both versions: drawn as the same laid page (the
/// very one, or one an insert or delete before it only moved in the flow, drawing the flow
/// items it drew, moved with it), in the same model context, page context and stories'
/// epoch, its body lines named alike -- what paints alike, digested or not.
fn same_page(
    (before, was): (&LaidVersion, &VersionPages),
    (now, is): (&LaidVersion, &VersionPages),
    page: usize,
) -> bool {
    before.epoch == now.epoch
        && before.context == now.context
        && was.contexts.get(page) == is.contexts.get(page)
        && was.ids.get(page) == is.ids.get(page)
        && matches!(
            (was.pages.get(page), is.pages.get(page)),
            (Some(Some(a)), Some(Some(b))) if a.drawn_alike(b)
        )
}
