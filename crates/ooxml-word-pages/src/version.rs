//! Owns: one version of a document laid out ([`LaidVersion`]): its layout, its facts made
//! once when first asked for, what its pages' SVG reads (its pictures, how faces are
//! named); and one page
//! as a reader sees it ([`PageView`]: docx2svg's `Page` -- its frame, its lines with their
//! spans, rules, pictures and drawings' slots, the body blocks it draws).
//!
//! Must not: lay out or paint (the engine does, under its owner's lock), or change once
//! made: a version is shared by every reader holding it.

use crate::pictures::PackagePictures;
use ooxml_word_layout::facts::LayoutFacts;
use ooxml_word_layout::geometry::PageFrame;
use ooxml_word_layout::paginate::PageInfo;
use ooxml_word_layout::paint::drawings::DrawingSlot;
use ooxml_word_layout::paint::{Drawn, FaceNames, PaintedLine, Rule};
use ooxml_word_layout::px::Px;
use ooxml_word_layout::{DocumentLayout, WordGlyphs};
use ooxml_word_model::value::Text;
use std::sync::{Arc, OnceLock};

/// One version laid out, immutable.
pub struct LaidVersion {
    pub(crate) layout: DocumentLayout,
    pub(crate) facts: OnceLock<LayoutFacts>,
    pub(crate) pictures: Option<PackagePictures>,
    pub(crate) drawings: Option<crate::drawings::WordDrawings>,
    pub(crate) glyphs: Arc<WordGlyphs>,
    pub(crate) names: Arc<dyn FaceNames>,
    pub(crate) relaid: bool,
    pub(crate) relaid_cause: Option<crate::engine::RelaidCause>,
    pub(crate) said: crate::warnings::Said,
    pub(crate) generation: u64,
}

impl std::fmt::Debug for LaidVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaidVersion")
            .field("pages", &self.layout.page_count())
            .field("relaid", &self.relaid)
            .finish_non_exhaustive()
    }
}

impl LaidVersion {
    /// The layout.
    pub fn layout(&self) -> &DocumentLayout {
        &self.layout
    }

    /// Its pages, blank ones included.
    pub fn page_count(&self) -> usize {
        self.layout.page_count()
    }

    /// What a check reads of it: made once, when first asked for.
    pub fn facts(&self) -> &LayoutFacts {
        self.facts.get_or_init(|| LayoutFacts::of(&self.layout))
    }

    /// Whether its facts were made already.
    pub fn has_facts(&self) -> bool {
        self.facts.get().is_some()
    }

    /// It was laid out from the version before (not whole).
    pub fn relaid(&self) -> bool {
        self.relaid
    }

    /// Why it was laid out whole although a version before was laid out, where it was.
    pub fn relaid_cause(&self) -> Option<&crate::engine::RelaidCause> {
        self.relaid_cause.as_ref()
    }

    /// Its warnings and approximations, docx2svg's.
    pub fn said(&self) -> &crate::warnings::Said {
        &self.said
    }

    /// The engine generation that laid it out: a version is painted by that engine only.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Its pictures.
    pub fn pictures(&self) -> Option<&PackagePictures> {
        self.pictures.as_ref()
    }

    /// How its SVG names faces.
    pub fn names(&self) -> &dyn FaceNames {
        self.names.as_ref()
    }

    /// Its faces.
    pub fn glyphs(&self) -> &WordGlyphs {
        &self.glyphs
    }

    /// What it holds, in bytes, roughly: its flow's items and pages, and its facts where
    /// made (each line and block counted at its size). A measure for a budget, not an
    /// allocator's count.
    pub fn weight(&self) -> usize {
        let pages = &self.layout.pagination;
        let lines: usize = pages.pages.iter().map(|p| p.lines.len() + p.rows.len()).sum();
        let facts = self.facts.get().map_or(0, |f| {
            f.pages
                .iter()
                .map(|p| p.lines.len() * 96 + p.blocks.iter().map(|b| b.len() + 16).sum::<usize>())
                .sum()
        });
        self.layout.flow.items.len() * 256 + pages.pages.len() * 512 + lines * 64 + facts
    }
}

/// One page as a reader sees it (docx2svg's `Page`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageView {
    /// 0-based, blank pages counted.
    pub page: usize,
    /// What the paginator knows of it.
    pub info: PageInfo,
    /// Its frame.
    pub frame: PageFrame,
    /// Its lines (docx2svg's `Line`): the body's, then its header's and footer's (their
    /// story's kind and part in `story`), then its footnotes'.
    pub lines: Vec<PaintedLine>,
    /// The rectangles the text needs.
    pub rules: Vec<Rule>,
    /// Its inline pictures.
    pub pictures: Vec<Drawn>,
    /// Its drawings, for their painters.
    pub drawings: Vec<DrawingSlot>,
    /// The body blocks it draws something of (`Page.blocks`).
    pub blocks: Vec<Text>,
    /// What its paint does not draw yet, by reason.
    pub unpainted: Vec<String>,
}

impl PageView {
    /// The text column's edges and the text area's: left, right, top, bottom (docx2svg's
    /// `Page.column`).
    pub fn column(&self) -> (Px, Px, Px, Px) {
        (self.frame.left, self.frame.right, self.frame.top, self.frame.bottom)
    }
}
