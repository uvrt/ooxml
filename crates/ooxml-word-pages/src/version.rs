//! Owns: one version of a document laid out ([`LaidVersion`]): its layout, its facts made
//! once when first asked for, what its pages' SVG reads (its pictures, how faces are
//! named); and one page
//! as a reader sees it ([`PageView`]: docx2svg's `Page` -- its frame, its lines with their
//! spans, rules, pictures and drawings' slots, the body blocks it draws).
//!
//! Must not: lay out or paint (the engine does, under its owner's lock), or change once
//! made: a version is shared by every reader holding it.

use crate::pictures::PackagePictures;
use ooxml_word_layout::facts::{LayoutFacts, PageFacts};
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
    /// The faces Word draws it with (what a rasteriser is handed).
    pub(crate) drawing: Arc<ooxml_fonts::DrawingFonts>,
    pub(crate) relaid: bool,
    pub(crate) relaid_cause: Option<crate::causes::RelaidCause>,
    pub(crate) said: crate::warnings::Said,
    pub(crate) generation: u64,
    /// Its number among the versions its engine laid out: what its pages' paint is kept
    /// under.
    pub(crate) serial: u64,
    /// Its body blocks by the tool's ids, named when asked.
    pub(crate) ids: crate::ids::VersionIds,
    /// Its summary, made once when first asked (its engine paints the pages it needs).
    pub(crate) summary: OnceLock<Arc<crate::summary::Summary>>,
    /// The stories' epoch of its generation it was laid out in.
    pub(crate) epoch: u64,
    /// The model's context of its generation it was laid out in: it is painted and measured
    /// in it, by the generation's model where that is its context still, else by a painter
    /// of its own.
    pub(crate) context: u64,
    /// Its context's mark: while a version of a context is held, its painter is kept.
    pub(crate) painted_in: Arc<crate::engine::ContextMark>,
    /// What a comparison reads of each page without painting it, made when first asked.
    pub(crate) pages: OnceLock<Arc<VersionPages>>,
}

/// What a comparison reads of a version's pages without painting them.
#[derive(Debug)]
pub(crate) struct VersionPages {
    /// Each page's context ([`crate::page_context`]).
    pub(crate) contexts: Vec<ooxml_base::Digest>,
    /// Each page as laid out (`None`: a page with no laid page of its own).
    pub(crate) pages: Vec<Option<crate::digest_cache::PageRef>>,
    /// Each page's body lines' paragraph paths, in drawing order.
    pub(crate) paths: Vec<Arc<[Text]>>,
    /// Each page's body lines' ids, in drawing order (`None`: a line of a paragraph the
    /// version does not name).
    pub(crate) ids: Vec<Vec<Option<Text>>>,
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
    /// The id of the paragraph, table or row at body path `path` (`w:body/w:p[3]`), as the
    /// tool names it in this version's bytes.
    pub fn id_of(&self, path: &str) -> Option<&str> {
        self.ids.all().get(path).map(|id| &**id)
    }

    /// Its body blocks by the tool's ids (every block named, once, when first asked).
    pub fn block_ids(&self) -> &crate::ids::BlockIds {
        self.ids.all()
    }

    /// The body path (`w:body/w:p[3]`) of the paragraph, table or row the tool's id `id`
    /// names in this version, found from the id alone.
    pub fn path_of(&self, id: &str) -> Option<Text> {
        self.ids.path_of(id)
    }

    /// The advance of `ch` in the face this version lays `family` out with, and that face's
    /// units per em (`ooxml_fonts::WordFonts::advance`: Word's glyph fallbacks included);
    /// `None` where nothing answers. Not recorded in the fonts' coverage: a measure for an
    /// edit, not a line of the layout.
    pub fn advance(&self, family: &str, bold: bool, italic: bool, ch: char) -> Option<(u32, u32)> {
        let found = self.glyphs.fonts().advance(family, bold, italic, ch)?;
        Some((u32::from(found.advance), u32::from(found.units_per_em)))
    }

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

    /// Page `page`'s facts (0-based, blank pages counted), on their own: the whole
    /// layout's where made already, else that page's alone, made once.
    pub fn page_facts(&self, page: usize) -> Option<&PageFacts> {
        match self.facts.get() {
            Some(all) => all.pages.get(page),
            None => self.layout.page_facts(page),
        }
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
    pub fn relaid_cause(&self) -> Option<&crate::causes::RelaidCause> {
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

    /// What it alone holds, in bytes, roughly: what dropping it would give back now
    /// (`crate::weight`: its own containers, the measures, pages and blocks no other
    /// version or memo holds), and its facts where made. A measure for a budget, not an
    /// allocator's count.
    pub fn weight(&self) -> usize {
        let facts = self.facts.get().map_or(0, crate::weight::facts_weight);
        crate::weight::layout_weight(&self.layout) + facts + self.ids.weight()
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
    /// Per drawing, the faces its text is drawn in (a text path's): `(face, bold, italic)`.
    pub drawing_faces: Vec<Vec<(String, bool, bool)>>,
    /// Per drawing, its text boxes' lines in page coordinates, in the order they are painted
    /// (docx2svg's `Float.lines`): what a reader of the lines a page draws reads beside
    /// `lines` (`Page.text_lines`).
    pub text_box_lines: Vec<Vec<PaintedLine>>,
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
