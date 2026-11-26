//! Owns: the measures a flow keeps per block between versions ([`MeasureCache`]): each
//! top-level paragraph's and table's, by the block's `Arc`, with what its measure depends
//! on beyond its content.
//!
//! Must not: measure (the flow does), or keep a block past the version that drops it.

use crate::paragraph::{MeasuredParagraph, Unmeasured};
use crate::table::TableFlow;
use ooxml_word_model::model::{Block, Paragraph, SectionProps, Table};
use std::collections::HashMap;
use std::sync::Arc;

/// What a block's measure depends on beyond its content: its section's geometry, its list
/// label and the parts it resolves against.
#[derive(Debug, Clone)]
pub(super) struct Kept {
    pub(super) paragraph: Arc<Paragraph>,
    pub(super) section: Arc<SectionProps>,
    pub(super) label: Option<String>,
    pub(super) measured: Result<Arc<MeasuredParagraph>, Unmeasured>,
}

/// Measures kept per block by `Arc`: the model shares an unchanged block's `Arc`
/// between versions wherever it stands (a block carries no place), and each entry holds its
/// block, so an address names one block for as long as it is kept. Valid for one set of
/// parts (styles, numbering, settings, theme) and faces: [`MeasureCache::clear`] when they
/// change.
#[derive(Debug, Default)]
pub struct MeasureCache {
    pub(super) blocks: HashMap<usize, Kept>,
    pub(super) tables: HashMap<usize, KeptTable>,
}

/// A table's flow and what it was measured from beyond the table: its section, the
/// paragraph before it and its cells' list labels.
#[derive(Debug, Clone)]
pub(super) struct KeptTable {
    pub(super) table: Arc<Table>,
    pub(super) section: Arc<SectionProps>,
    pub(super) previous: Option<Arc<Paragraph>>,
    pub(super) labels: Vec<Option<String>>,
    pub(super) flow: Arc<dyn TableFlow>,
}

impl MeasureCache {
    /// Forget everything.
    pub fn clear(&mut self) {
        self.blocks.clear();
        self.tables.clear();
    }

    /// How many blocks are kept.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether nothing is kept.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub(super) fn get(
        &mut self,
        paragraph: &Arc<Paragraph>,
        section: &Arc<SectionProps>,
        label: Option<&str>,
    ) -> Option<Result<Arc<MeasuredParagraph>, Unmeasured>> {
        let fits = |kept: &Kept| {
            (Arc::ptr_eq(&kept.section, section) || kept.section == *section) && kept.label.as_deref() == label
        };
        let kept = self.blocks.get(&key_of(paragraph))?;
        (Arc::ptr_eq(&kept.paragraph, paragraph) && fits(kept)).then(|| kept.measured.clone())
    }

    /// Keep `kept`'s measure, by its paragraph.
    pub(super) fn insert(&mut self, kept: Kept) {
        self.blocks.insert(key_of(&kept.paragraph), kept);
    }

    /// Keep only the top-level paragraphs of `bodies` (a new version's, each pass's):
    /// the rest are dropped.
    pub fn retain_blocks(&mut self, bodies: &[&[Block]]) {
        let live: std::collections::HashSet<usize> = bodies
            .iter()
            .flat_map(|body| body.iter())
            .map(|b| match b {
                Block::Paragraph(p) => Arc::as_ptr(p) as usize,
                Block::Table(t) => Arc::as_ptr(t) as usize,
            })
            .collect();
        self.blocks.retain(|k, _| live.contains(k));
        self.tables.retain(|k, _| live.contains(k));
    }
}

/// A paragraph's key in [`MeasureCache::blocks`]: its `Arc`'s address.
fn key_of(paragraph: &Arc<Paragraph>) -> usize {
    Arc::as_ptr(paragraph) as usize
}
