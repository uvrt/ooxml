//! Owns: a page as a check reads it ([`PageEngine::page_read`]): its entry in the page
//! tier (its read and its facts' contributions), taken from the generation's
//! [`crate::page_reads::ReadMemo`] where a version before read the page laid out the same,
//! in the same context, of the same blocks at the same places; else painted (not kept) and read, and the
//! entry kept. And each page's context ([`PageEngine::page_contexts`]), made
//! once per version: what a page's read and its digests depend on beyond its laid page.
//!
//! Must not: keep a page's paint for the read (the paint memo keeps a reader's pages).

use super::painters::Painter;
use super::{PageEngine, KEY};
use crate::laid_entry::LaidEntry;
use crate::page_context::PageContext;
use crate::page_entry::PageEntry;
use crate::page_read::PageRead;
use crate::page_reads::{page_keys, places, Found, ReadOf, Split};
use crate::text_boxes::{text_boxes, TextBoxJob};
use crate::version::LaidVersion;
use ooxml_word_layout::paint::{paint_stories, PaintContext};
use ooxml_word_model::model::Table;
use ooxml_word_model::resolve::ResolvedTable;
use std::sync::Arc;

impl PageEngine {
    /// Page `page` of `version` as a check reads it ([`crate::page_read`]): its lines' text
    /// and the boxes beside them, from one paint (not kept), or as a version before read
    /// the page where it is laid out the same in the same context at the same places --
    /// with only its stories painted again where they show other values.
    pub fn page_read(&mut self, version: &LaidVersion, page: usize) -> Option<Arc<PageEntry>> {
        let key = crate::page_said::page_key(&version.layout, page);
        let context = match &key {
            Some(_) => self.page_contexts(version)?.get(page).copied(),
            None => None,
        };
        let of = key.as_ref().zip(context).map(|(key, context)| ReadOf {
            page: key,
            context,
            epoch: version.epoch,
            model: version.context,
            places: places(&version.layout, &key.items),
            keys: page_keys(&version.layout, &key.items),
        });
        let Some(of) = of else {
            let paint = self.painted_page(version, page, false)?;
            self.stats.pages_read.0 += 1;
            return Some(Arc::new(PageEntry::new(PageRead::of(
                &paint,
                &version.layout.document.names,
            ))));
        };
        match self.generation.as_mut().and_then(|g| g.reads.get(&of)) {
            Some(Found::Whole(entry)) => {
                self.stats.pages_read.1 += 1;
                Some(entry)
            }
            Some(Found::Body(kept, split)) => {
                let read = split.joined(kept.read(), self.stories_read(version, page)?);
                self.stats.pages_read.2 += 1;
                Some(self.keep_read(&of, read, Some(split)))
            }
            None => {
                let paint = self.painted_page(version, page, false)?;
                let read = PageRead::of(&paint, &version.layout.document.names);
                self.stats.pages_read.0 += 1;
                let split = self.stories_read(version, page).and_then(|s| Split::of(&read, &s));
                Some(self.keep_read(&of, read, split))
            }
        }
    }

    /// Page `page` of `version`'s entry in the layout half of the page tier
    /// ([`crate::laid_entry`]): its facts' contributions made from its layout facts, as a
    /// version before made them where it shares the page's facts; else made and kept.
    /// `None` past the last page.
    pub fn laid_entry(&mut self, version: &LaidVersion, page: usize) -> Option<Arc<LaidEntry>> {
        let facts = version.facts().pages.get(page)?;
        match self.generation.as_mut() {
            Some(generation) => Some(generation.laid.entry(facts)),
            None => Some(Arc::new(LaidEntry::of(facts))),
        }
    }

    /// What the layout half of the page tier holds now (on the paint budget's bounded
    /// account): its entries and bytes, the lookups it answered and missed.
    pub fn laid_tier(&self) -> ooxml_derive::MemoStats {
        self.generation.as_ref().map(|g| g.laid.stats()).unwrap_or_default()
    }

    /// `read` as the page `of` names, its entry kept in the page tier.
    fn keep_read(&mut self, of: &ReadOf<'_>, read: PageRead, split: Option<Split>) -> Arc<PageEntry> {
        let entry = Arc::new(PageEntry::new(read));
        if let Some(generation) = self.generation.as_mut() {
            generation.reads.insert(of, Arc::clone(&entry), split);
        }
        entry
    }

    /// Page `page` of `version`'s stories (its header, footer and footnotes) painted on
    /// their own, their text boxes too, as a check reads them.
    fn stories_read(&mut self, version: &LaidVersion, page: usize) -> Option<PageRead> {
        let (glyph_size, decimal) = (self.glyph_size, self.decimal);
        let Painter { model, layout: stories } =
            self.generation
                .as_mut()?
                .painter(version, &self.locale, &self.budgets.held)?;
        let parts = stories.page_parts(model, &version.layout, page);
        let cx = PaintContext {
            faces: version.glyphs.as_ref(),
            names: version.names.as_ref(),
            glyph_size,
            decimal,
        };
        let mut paint = paint_stories(&version.layout, model, page, &parts, &cx)?;
        if let Some(drawings) = &version.drawings {
            let job = TextBoxJob {
                layout: stories,
                model,
                laid: &version.layout,
                cx: &cx,
            };
            paint.text_boxes = text_boxes(job, drawings, &paint, page);
        }
        Some(PageRead::of(&paint, &version.layout.document.names))
    }

    /// A table of `version`'s layout model (nested ones too) resolved as the layout
    /// resolved it: from the model's table memo (the block tier: by the table's content,
    /// in the version's own context), shared by the versions that keep the table and by
    /// the layout, which resolved it already. `None` where this engine no longer lays the
    /// version's context out.
    pub fn resolved_table(&mut self, version: &LaidVersion, table: &Table) -> Option<Arc<ResolvedTable>> {
        let painter = self
            .generation
            .as_mut()?
            .painter(version, &self.locale, &self.budgets.held)?;
        Some(painter.model.resolve_table(&version.layout.document, table))
    }

    /// What the page tier holds now (its entries: each page's read and its facts'
    /// contributions, on the paint budget's bounded account): its entries and bytes, the
    /// lookups it answered and missed.
    pub fn page_tier(&self) -> ooxml_derive::MemoStats {
        self.generation.as_ref().map(|g| g.reads.stats()).unwrap_or_default()
    }

    /// Each page's context of `version` ([`crate::page_context`]), made once; `None` where
    /// this engine no longer paints it.
    pub(crate) fn page_contexts(&mut self, version: &LaidVersion) -> Option<Arc<[PageContext]>> {
        if let Some(found) = version.unpainted.contexts.get() {
            return Some(Arc::clone(found));
        }
        if !self.paints(version) {
            return None;
        }
        let painter = self
            .generation
            .as_mut()?
            .painter(version, &self.locale, &self.budgets.held)?;
        let made: Arc<[PageContext]> =
            crate::page_context::page_contexts(&version.layout, painter.model, &self.locale, KEY).into();
        Some(Arc::clone(version.unpainted.contexts.get_or_init(|| made)))
    }
}
