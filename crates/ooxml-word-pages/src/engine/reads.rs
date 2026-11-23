//! Owns: a page as a check reads it ([`PageEngine::page_read`]), taken from the
//! generation's [`crate::page_reads::ReadMemo`] where a version before read the page laid
//! out the same, in the same context and at the same places; else painted (not kept) and
//! read, and the read kept. And each page's context ([`PageEngine::page_contexts`]), made
//! once per version: what a page's read and its digests depend on beyond its laid page.
//!
//! Must not: keep a page's paint for the read (the paint memo keeps a reader's pages).

use super::painters::Painter;
use super::{PageEngine, KEY};
use crate::page_context::PageContext;
use crate::page_read::PageRead;
use crate::page_reads::{places, Found, ReadOf, Split};
use crate::text_boxes::{text_boxes, TextBoxJob};
use crate::version::LaidVersion;
use ooxml_word_layout::paint::{paint_stories, PaintContext};
use std::sync::Arc;

impl PageEngine {
    /// Page `page` of `version` as a check reads it ([`crate::page_read`]): its lines' text
    /// and the boxes beside them, from one paint (not kept), or as a version before read
    /// the page where it is laid out the same in the same context at the same places --
    /// with only its stories painted again where they show other values.
    pub fn page_read(&mut self, version: &LaidVersion, page: usize) -> Option<PageRead> {
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
        });
        let Some(of) = of else {
            let paint = self.painted_page(version, page, false)?;
            self.stats.pages_read.0 += 1;
            return Some(PageRead::of(&paint));
        };
        match self.generation.as_mut().and_then(|g| g.reads.get(&of)) {
            Some(Found::Whole(read)) => {
                self.stats.pages_read.1 += 1;
                Some(PageRead::clone(&read))
            }
            Some(Found::Body(kept, split)) => {
                let read = split.joined(&kept, self.stories_read(version, page)?);
                self.stats.pages_read.2 += 1;
                self.keep_read(&of, &read, Some(split));
                Some(read)
            }
            None => {
                let paint = self.painted_page(version, page, false)?;
                let read = PageRead::of(&paint);
                self.stats.pages_read.0 += 1;
                let split = self.stories_read(version, page).and_then(|s| Split::of(&read, &s));
                self.keep_read(&of, &read, split);
                Some(read)
            }
        }
    }

    fn keep_read(&mut self, of: &ReadOf<'_>, read: &PageRead, split: Option<Split>) {
        if let Some(generation) = self.generation.as_mut() {
            generation.reads.insert(of, Arc::new(read.clone()), split);
        }
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
        Some(PageRead::of(&paint))
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
