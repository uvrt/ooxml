//! Owns: a page as a check reads it ([`PageEngine::page_read`]), taken from the
//! generation's [`crate::page_reads::ReadMemo`] where a version before read the page laid
//! out the same, in the same context and at the same places; else painted (not kept) and
//! read, and the read kept. And each page's context ([`PageEngine::page_contexts`]), made
//! once per version: what a page's read and its digests depend on beyond its laid page.
//!
//! Must not: keep a page's paint for the read (the paint memo keeps a reader's pages).

use super::{PageEngine, KEY};
use crate::page_read::PageRead;
use crate::page_reads::{places, ReadOf};
use crate::version::LaidVersion;
use ooxml_base::Digest;
use std::sync::Arc;

impl PageEngine {
    /// Page `page` of `version` as a check reads it ([`crate::page_read`]): its lines' text
    /// and the boxes beside them, from one paint (not kept), or as a version before read
    /// the page where it is laid out the same in the same context at the same places.
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
        if let Some(of) = &of {
            let found = self.generation.as_mut().and_then(|g| g.reads.get(of));
            if let Some(found) = found {
                self.stats.pages_read.1 += 1;
                return Some(PageRead::clone(&found));
            }
        }
        let paint = self.painted_page(version, page, false)?;
        let read = PageRead::of(&paint);
        self.stats.pages_read.0 += 1;
        if let (Some(of), Some(generation)) = (&of, self.generation.as_mut()) {
            generation.reads.insert(of, Arc::new(read.clone()));
        }
        Some(read)
    }

    /// Each page's context of `version` ([`crate::page_context`]), made once; `None` where
    /// this engine no longer paints it.
    pub(crate) fn page_contexts(&mut self, version: &LaidVersion) -> Option<Arc<[Digest]>> {
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
        let made: Arc<[Digest]> =
            crate::page_context::page_contexts(&version.layout, painter.model, &self.locale, KEY).into();
        Some(Arc::clone(version.unpainted.contexts.get_or_init(|| made)))
    }
}
