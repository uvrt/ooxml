//! Owns: the layout's model of one document's versions ([`Model`]): each version projected
//! from its reading in the final view ([`Model::project`], [`crate::projection::Projection`])
//! with its body's list labels kept up to date across versions (a label tree changed by the
//! blocks that changed, in O(log n) each), and its blocks' resolved formats, memoised while
//! the styles, numbering, settings and theme are the same.
//!
//! Must not: read a part (the projection does), hold more than one document's state, or
//! anything global; the caller owns the `Model` (the document's derived engine, design
//! section 6).

use crate::document::{Context, ResolvedParagraph, WordDocument};
use crate::docx::Docx;
use crate::error::WordError;
use crate::ids::VersionDelta;
use crate::labels::BlockItems;
use crate::model::{Block, Paragraph, Table};
use crate::projection::Projection;
use crate::resolve::{ResolvedTable, Resolver};
use crate::value::Props;
use crate::view::View;
use ooxml_base::{Digest, DigestKey};
use ooxml_derive::{CacheBudget, Memo, MemoStats, MonoidTree};
use std::sync::Arc;

mod labels;

/// What the last projection did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenStats {
    /// Top-level body elements walked (none where the body was taken whole).
    pub elements: usize,
    /// Of them, read (not taken from the block tier).
    pub elements_read: usize,
    /// Body blocks whose list items were found again.
    pub label_blocks: usize,
    /// Edits made to the labels' tree.
    pub label_edits: usize,
    /// Resolutions computed (paragraphs, runs, tables), not taken from the memo.
    pub resolutions: u64,
    /// Whether the styles, numbering, settings or theme changed (everything resolved again).
    pub context_changed: bool,
}

/// The layout's model of one document's versions.
pub struct Model {
    key: DigestKey,
    budget: Arc<dyn CacheBudget>,
    /// The context the last version resolved against.
    context: Option<Context>,
    resolver: Resolver,
    /// The last version's body, and each of its blocks' label key.
    labelled: Option<(Arc<[Block]>, Vec<Digest>)>,
    labels: MonoidTree<BlockItems>,
    label_memo: Memo<Digest, BlockItems>,
    stats: OpenStats,
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model").field("stats", &self.stats).finish()
    }
}

impl Model {
    /// A model for one document, its caches reserving from `budget`, its content digests
    /// keyed by `key` (the session's).
    pub fn new(budget: Arc<dyn CacheBudget>, key: DigestKey) -> Model {
        Model {
            key,
            context: None,
            resolver: Resolver::new(&budget),
            labelled: None,
            labels: MonoidTree::new(),
            label_memo: Memo::new(Arc::clone(&budget)),
            stats: OpenStats::default(),
            budget,
        }
    }

    /// What the last [`Model::project`] did.
    pub fn stats(&self) -> OpenStats {
        self.stats
    }

    /// Drop what the memos hold that no version projected since the last sweep used and
    /// nothing else holds ([`ooxml_derive::Memo::sweep`]): called once a version is
    /// projected and laid out, it keeps the memos at what the versions in use need. List
    /// items and resolutions stay while a version holds them. Returns how many entries were
    /// dropped.
    pub fn sweep(&mut self) -> usize {
        self.label_memo.sweep(|items| Arc::strong_count(&items.items) > 1) + self.resolver.sweep()
    }

    /// The resolution memo's counters.
    pub fn resolver_stats(&self) -> MemoStats {
        self.resolver.stats()
    }

    /// The budget the caches reserve from.
    pub fn budget(&self) -> &Arc<dyn CacheBudget> {
        &self.budget
    }

    /// The version `version` (a reading of this model's document) as the layout reads it:
    /// projected in the final view ([`Projection::counted`]; `delta`, what it changed
    /// against its base) with its body's list labels.
    ///
    /// # Errors
    /// As [`Projection::of`].
    pub fn project(&mut self, version: &Docx, delta: Option<&VersionDelta>) -> Result<WordDocument, WordError> {
        let misses_before = self.resolver.stats().misses;
        let (mut document, projected) = Projection::counted(version, delta, View::Final)?;
        let changed = self.context.as_ref().is_none_or(|known| !known.same(&document.context));
        if changed {
            self.resolver.clear();
            self.label_memo.clear();
            self.labelled = None;
            self.labels = MonoidTree::new();
            self.context = Some(document.context.clone());
        }
        self.stats.context_changed = changed;
        self.update_labels(&document.context, &document.body);
        document.labels = self.labels.clone();
        self.stats.elements = projected.elements;
        self.stats.elements_read = projected.elements_read;
        self.stats.resolutions = self.resolver.stats().misses - misses_before;
        Ok(document)
    }

    /// A paragraph's resolved formats in `document` (a version this model projected).
    pub fn resolve_paragraph(&mut self, document: &WordDocument, paragraph: &Paragraph) -> ResolvedParagraph {
        self.resolver.paragraph(&document.context, paragraph)
    }

    /// A paragraph's own resolved properties.
    pub fn resolve_paragraph_props(&mut self, document: &WordDocument, paragraph: &Paragraph) -> Arc<Props> {
        self.resolver.paragraph_props(&document.context, paragraph)
    }

    /// A table's resolved formats in `document`.
    pub fn resolve_table(&mut self, document: &WordDocument, table: &Table) -> Arc<ResolvedTable> {
        self.resolver.table(&document.context, table)
    }
}
