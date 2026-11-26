//! Owns: one version of a document laid out ([`LaidVersion`]): its layout, its facts made
//! once when first asked for, what its pages' SVG reads (its pictures, how faces are
//! named); and one page
//! as a reader sees it ([`PageView`]: docx2svg's `Page` -- its frame, its lines with their
//! spans, rules, pictures and drawings' slots, the body blocks it draws).
//!
//! Must not: lay out or paint (the engine does, under its owner's lock), or change once
//! made: a version is shared by every reader holding it.

use crate::line_text::{ItemPlace, LineText};
use crate::pictures::PackagePictures;
use ooxml_word_layout::facts::{LayoutFacts, PageFacts};
use ooxml_word_layout::geometry::PageFrame;
use ooxml_word_layout::paginate::PageInfo;
use ooxml_word_layout::paint::drawings::DrawingSlot;
use ooxml_word_layout::paint::{Drawn, FaceNames, PaintedLine, Rule};
use ooxml_word_layout::px::Px;
use ooxml_word_layout::{DocumentLayout, WordGlyphs};
use ooxml_word_model::docx::{Docx, StoryPart, VersionNames};
use ooxml_word_model::ids::{Anchor, BlockKey, StoryIx, Sub};
use ooxml_word_model::value::Text;
use ooxml_xml::Element;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// One version laid out, immutable.
pub struct LaidVersion {
    pub(crate) layout: DocumentLayout,
    pub(crate) facts: OnceLock<LayoutFacts>,
    /// The version its facts are made from when first asked (the pages both share take
    /// theirs from it), let go once they are made ([`crate::engine`]'s `facts_source`).
    pub(crate) facts_from: Mutex<Option<Arc<LaidVersion>>>,
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
    /// What is read of its pages without painting them, made when first asked.
    pub(crate) unpainted: Unpainted,
}

/// What is read of a version's pages without painting them, each made when first asked.
#[derive(Debug, Default)]
pub(crate) struct Unpainted {
    /// What a comparison reads of each page.
    pub(crate) pages: OnceLock<Arc<VersionPages>>,
    /// Each page's context ([`crate::page_context`]).
    pub(crate) contexts: OnceLock<Arc<[crate::page_context::PageContext]>>,
}

/// What a comparison reads of a version's pages without painting them.
#[derive(Debug)]
pub(crate) struct VersionPages {
    /// Each page's context ([`crate::page_context`]).
    pub(crate) contexts: Arc<[crate::page_context::PageContext]>,
    /// Each page as laid out (`None`: a page with no laid page of its own).
    pub(crate) pages: Vec<Option<crate::digest_cache::PageRef>>,
    /// Each page's blocks' keys as its read is checked by
    /// ([`crate::page_reads::page_blocks`]; `None`: a page with no laid page of its own).
    pub(crate) blocks: Vec<Option<crate::page_reads::PageBlocks>>,
    /// Each page's body lines' paragraph keys, in drawing order (`None`: a paragraph no
    /// story's index holds).
    pub(crate) keys: Vec<Arc<[Option<BlockKey>]>>,
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
    /// Its names: its story parts' indexes, which its blocks' keys are named through.
    fn block_names(&self) -> &VersionNames {
        &self.layout.document.names
    }

    /// The id of the paragraph, table or row at body path `path` (`w:body/w:p[3]`), as the
    /// tool names it in this version's bytes: the path's block found through the version's
    /// names ([`VersionNames::key_at_path`]), never one inside a paragraph's content.
    pub fn id_of(&self, path: &str) -> Option<String> {
        let names = self.block_names();
        let key = names.key_at_path(StoryIx::BODY, path)?;
        names.id(key).map(|id| id.to_string())
    }

    /// The id the tool names a body block by in this version, through the version's names
    /// from its key `key`; a block with no key, or not the body's, by its body path `path`
    /// ([`LaidVersion::id_of`]).
    pub fn id_named(&self, key: Option<BlockKey>, path: &str) -> Option<Text> {
        let Some(key) = key.filter(|k| k.story() == StoryIx::BODY) else {
            return self.id_of(path).map(|id| Text::from(id.as_str()));
        };
        self.block_names().id(key).map(|id| Text::from(id.to_string().as_str()))
    }

    /// Whether a body block the tool names in this version is `key`'s, without naming it:
    /// through the version's names; a block with no key, or not the body's, by its body path
    /// `path` ([`LaidVersion::is_named`]).
    pub fn is_named_by(&self, key: Option<BlockKey>, path: &str) -> bool {
        match key.filter(|k| k.story() == StoryIx::BODY) {
            Some(key) => self.block_names().holds(key),
            None => self.is_named(path),
        }
    }

    /// The body blocks page `facts` draws something of, as written (`Page.blocks`): named
    /// through the version's names from their anchors where every one is anchored (each its
    /// path, asserted in debug builds), else their paths.
    pub fn page_blocks(&self, facts: &PageFacts) -> Vec<String> {
        if facts.anchors.len() != facts.blocks.len() {
            return facts.blocks.iter().map(ToString::to_string).collect();
        }
        let names = self.block_names();
        facts
            .anchors
            .iter()
            .zip(&facts.blocks)
            .map(|(anchor, path)| {
                let named = names.path(*anchor);
                debug_assert_eq!(named.as_deref(), Some(&**path), "{anchor:?} is named as its path");
                named.unwrap_or_else(|| path.to_string())
            })
            .collect()
    }

    /// Whether the body path `path` names a paragraph, table or row the tool names in this
    /// version (`id_of` is `Some`), without naming it.
    pub fn is_named(&self, path: &str) -> bool {
        self.block_names().key_at_path(StoryIx::BODY, path).is_some()
    }

    /// The tool's ids of the body's top-level blocks (paragraphs and tables, those of its
    /// block-level content controls too) page `page` (0-based) draws something of, each once,
    /// in document order (docx-agent's `DocumentLayout.blocks_on`): a block split across
    /// pages is on each; empty past the last page. Found from the keys of its lines'
    /// paragraphs through the version's names; a block the version does not name is left
    /// out.
    pub fn top_blocks(&self, page: usize) -> Vec<String> {
        let Some(facts) = self.page_facts(page) else {
            return Vec::new();
        };
        let names = self.block_names();
        let mut seen = std::collections::HashSet::new();
        let mut found: Vec<(ooxml_word_model::ids::Path, String)> = facts
            .lines
            .iter()
            .filter_map(|line| line.anchor.map(|a| a.block))
            .filter(|key| key.story() == StoryIx::BODY && seen.insert(*key))
            .filter_map(|key| {
                let top = names.top_block_of(key)?;
                Some((names.place_of(top)?, names.id(top)?.to_string()))
            })
            .collect();
        found.sort();
        found.dedup_by(|a, b| a.1 == b.1);
        found.into_iter().map(|(_, id)| id).collect()
    }

    /// The body path (`w:body/w:p[3]`) of the paragraph, table or row the tool's id `id`
    /// names in this version, found from the id alone through the version's names.
    pub fn path_of(&self, id: &str) -> Option<Text> {
        let names = self.block_names();
        let key = names.key_of_id(StoryIx::BODY, id)?;
        names.path(Anchor::whole(key)).map(|path| Text::from(path.as_str()))
    }

    /// Where the paragraph the id `id` names is inside a body paragraph's content (a text
    /// box's paragraph, which the body's lines do not draw): the body path of the paragraph
    /// holding it (`w:body/w:p[2]`), and its own place (child indexes from the root, every
    /// node counted), which a painted line's path is matched with
    /// ([`LaidVersion::place_of_path`]). `None` for any other id.
    pub fn in_paragraph(&self, id: &str) -> Option<(Text, Vec<u32>)> {
        let names = self.block_names();
        let part = names.part(StoryIx::BODY)?;
        let Some(ooxml_word_model::ids::Entry::Paragraph(k)) = part.ids.get(id) else {
            return None;
        };
        let place = part.ids.paragraphs().get(k)?.path;
        // The first paragraph on the way down (below the body) holds it.
        let mut node = part.root().children().get(*place.first()? as usize)?.as_element()?;
        for (depth, step) in place.iter().enumerate().skip(1) {
            node = node.children().get(*step as usize)?.as_element()?;
            if node.name().is(Some(ooxml_word_model::ids::W), "p") {
                if depth + 1 == place.len() {
                    return None;
                }
                let host = part.ids.key_at(StoryIx::BODY, place.get(..=depth)?)?;
                let path = names.path(Anchor::whole(host))?;
                return Some((Text::from(path.as_str()), place.to_vec()));
            }
        }
        None
    }

    /// The place (child indexes from the root, every node counted) of the element a layout
    /// path names in the main part (a text box's paragraph's
    /// `…/w:r[2]/wp:anchor[1]/wps:txbx/w:txbxContent/w:p[1]` too).
    pub fn place_of_path(&self, path: &str) -> Option<Vec<u32>> {
        let (_, place) = self.block_names().part(StoryIx::BODY)?.resolve_path(path)?;
        Some(place)
    }

    /// The story part (of the version's reading `docx`), element and place (child indexes
    /// from the part's root) of what a page read is drawn for at `place`; `part`, the story
    /// part of a header's or footer's line drawn for no block its index holds.
    pub fn element_of<'d>(
        &self,
        docx: &'d Docx,
        place: &ItemPlace,
        part: Option<&str>,
    ) -> Option<(&'d StoryPart, &'d Arc<Element>, Vec<u32>)> {
        let names = self.block_names();
        let story = match (place.key(), place) {
            (Some(key), _) => docx.part(&names.part(key.story())?.part)?,
            (None, ItemPlace::Path(path)) => match part {
                Some(part) => docx.part(part)?,
                None => match crate::layout_facts::story_kind(path) {
                    "footnotes" => docx.story("footnotes"),
                    "endnotes" => docx.story("endnotes"),
                    _ => None,
                }
                .or_else(|| docx.body())?,
            },
            (None, _) => return None,
        };
        if let ItemPlace::Anchored(anchor) = place {
            if anchor.sub == Sub::Whole {
                let at = names.place_of(anchor.block)?.to_vec();
                let element = ooxml_word_model::docx::at_path(story.root(), &at)?;
                return Some((story, element, at));
            }
        }
        let (element, at) = story.resolve_path(&place.named(names)?)?;
        Some((story, element, at))
    }

    /// The tool's id of the paragraph a page read's `line` is of, in its story's part (a
    /// text box's paragraph's too), the version's reading being `docx`.
    pub fn line_paragraph_id(&self, docx: &Docx, line: &LineText) -> Option<String> {
        let part = line.story.as_ref().map(|(_, part)| part.as_str());
        let (story, _, place) = self.element_of(docx, &line.place, part)?;
        story.ids.paragraph_at(&place).map(|p| p.id)
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

    /// What a check reads of it: made once, when first asked for -- the pages it shares
    /// with the version its facts are made from taking theirs from that one's
    /// ([`LayoutFacts::after`]), the rest made ([`LayoutFacts::of`]). Never made while it
    /// is laid out.
    pub fn facts(&self) -> &LayoutFacts {
        self.facts.get_or_init(|| {
            let from = self.facts_from.lock().unwrap_or_else(PoisonError::into_inner).take();
            match from.as_deref().and_then(|v| Some((v, v.facts.get()?))) {
                Some((before, known)) => LayoutFacts::after(&self.layout, &before.layout, known),
                None => LayoutFacts::of(&self.layout),
            }
        })
    }

    /// The version its facts are to be made from, while they are not made.
    pub(crate) fn facts_source(&self) -> Option<Arc<LaidVersion>> {
        self.facts_from.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Page `page`'s facts (0-based, blank pages counted), on their own: the whole
    /// layout's where made already, else that page's alone, made once.
    pub fn page_facts(&self, page: usize) -> Option<&PageFacts> {
        match self.facts.get() {
            Some(all) => all.pages.get(page).map(|p| &**p),
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
        crate::weight::layout_weight(&self.layout) + facts
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
