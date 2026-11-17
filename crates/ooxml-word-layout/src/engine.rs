//! Owns: [`WordLayout`], one document's layout engine: what it keeps between versions
//! (measures by block and by content, for the body and for the stories), and laying a
//! version out -- its stories' heights, its flow, its pages -- into a [`DocumentLayout`].
//!
//! Must not: hold more than one document's state, or anything global; edit; draw.

use crate::faces::Faces;
use crate::fields::styleref::{style_references, StyleSearch};
use crate::fields::{cross_references, field_values, page_fields, substitute, Computed, FieldValues, Substitutions};
use crate::flow::{Flow, FlowContext, FlowMaker, FlowState, Labels};
use crate::incremental::{changes, repaginate, RelayoutStats};
use crate::notes::{footnotes_by_page, with_endnotes, with_footnotes, FootnotePages, NoteRole};
use crate::page_stories::{page_stories, PageStoryJob, StoryMemo};
use crate::paginate::{HyphenMove, PageContext, PageStories, Pagination, Paginator, Resume};
use crate::sections::Sections;
use crate::stories::{StoryContext, StoryTable};
use crate::table::TableLayout;
use ooxml_base::{DigestKey, Locale};
use ooxml_derive::CacheBudget;
use ooxml_text::Measurer;
use ooxml_word_model::{Model, WordDocument};
use std::sync::Arc;

/// One version laid out.
#[derive(Debug, Clone)]
pub struct DocumentLayout {
    /// The body's flow.
    pub flow: Flow,
    /// The sections.
    pub sections: Sections,
    /// The headers' and footers' heights.
    pub stories: StoryTable,
    /// The pages whose header or footer a `STYLEREF` gives heights of their own, as this
    /// pass was laid out with them.
    pub page_stories: PageStories,
    /// The pages.
    pub pagination: Pagination,
    /// The document is in compatibility mode 15 or later.
    pub mode15: bool,
    /// What Word does with a page ending in an automatic hyphen.
    pub hyphen_at_page_end: HyphenMove,
    /// The version as laid out: its page-number fields holding [`DocumentLayout::fields`].
    pub document: WordDocument,
    /// The page-number fields' values it was laid out with.
    pub fields: FieldValues,
    /// Per top-level block of [`DocumentLayout::document`], what of the endnotes placed
    /// in the flow it is.
    pub roles: Arc<[Option<NoteRole>]>,
    /// Where footnotes restart on each page: the page each note's reference was on in the
    /// pass before, which numbered them.
    pub footnote_pages: Option<FootnotePages>,
    /// The passes laid out until the fields settled.
    pub passes: usize,
    /// Whether they settled within [`FIELD_PASSES`] (else the last pass is kept).
    pub settled: bool,
    /// The passes before the last, each as laid out: what a relayout of the next version
    /// lays the same pass out again from.
    pub earlier: Vec<PassRecord>,
    /// What each pass's relayout did (empty for a full layout).
    pub relaid: Vec<RelayoutStats>,
}

/// One pass of the field passes as laid out.
#[derive(Debug, Clone)]
pub struct PassRecord {
    /// Its flow.
    pub flow: Flow,
    /// Its pages.
    pub pagination: Pagination,
    /// The pages whose header or footer had heights of their own.
    pub page_stories: PageStories,
}

impl DocumentLayout {
    /// Pass `k`'s flow and pages (the last pass is the layout's own).
    pub fn pass(&self, k: usize) -> Option<(&Flow, &Pagination)> {
        self.laid_pass(k).map(|(flow, pages, _)| (flow, pages))
    }

    /// Pass `k`'s flow, pages and pages with stories of their own heights.
    fn laid_pass(&self, k: usize) -> Option<(&Flow, &Pagination, &PageStories)> {
        match self.earlier.get(k) {
            Some(p) => Some((&p.flow, &p.pagination, &p.page_stories)),
            None if k == self.earlier.len() => Some((&self.flow, &self.pagination, &self.page_stories)),
            None => None,
        }
    }
}

/// At most this many layouts of the body until its page-number fields settle.
pub const FIELD_PASSES: usize = 4;

impl DocumentLayout {
    /// The context its pages are laid out in, paragraphs broken again with `breaker`'s
    /// faces.
    pub fn page_context<'s>(&'s self, faces: &'s dyn Faces, locale: &'s Locale) -> PageContext<'s> {
        PageContext {
            items: &self.flow.items,
            sections: &self.sections,
            mode15: self.mode15,
            hyphen_at_page_end: self.hyphen_at_page_end,
            stories: &self.stories,
            page_stories: &self.page_stories,
            separators: &self.flow.separators,
            note_items: &self.flow.note_items,
            notes_at: &self.flow.notes_at,
            wraps: self.flow.wraps,
            breaker: crate::rebreak::Breaker {
                document: &self.document,
                faces,
                locale,
            },
            lines: std::cell::RefCell::default(),
        }
    }
}

/// What Word does with a page whose last line would end in an automatic hyphen
/// (`paginate.hyphen_at_page_end`): in mode 15 the line moves unless the settings keep it
/// or move only the word; below mode 15 nothing moves.
pub fn hyphen_at_page_end(document: &WordDocument) -> HyphenMove {
    let settings = &document.context.settings;
    if settings.compatibility_mode.unwrap_or(0) < 15 {
        return HyphenMove::Nothing;
    }
    let on = |v: &str| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "on");
    if settings.compat_setting("allowHyphenationAtTrackBottom").is_some_and(on) {
        return HyphenMove::Nothing;
    }
    match settings.compat_setting("useWord2013TrackBottomHyphenation") {
        None => HyphenMove::Line,
        Some(v) if on(v) => HyphenMove::Line,
        Some(_) => HyphenMove::Word,
    }
}

/// One document's layout engine.
pub struct WordLayout {
    faces: Arc<dyn Faces>,
    tables: Arc<dyn TableLayout>,
    locale: Locale,
    body: FlowState,
    stories: FlowState,
    substitutions: Substitutions,
    style_values: Substitutions,
    computed: Computed,
}

impl WordLayout {
    /// An engine measuring with `faces`, laying tables out with `tables`, keeping its
    /// measures within `budget`, digesting content under `key`.
    pub fn new(
        faces: Arc<dyn Faces>,
        tables: Arc<dyn TableLayout>,
        locale: Locale,
        budget: &Arc<dyn CacheBudget>,
        key: DigestKey,
    ) -> WordLayout {
        WordLayout {
            faces,
            tables,
            locale,
            body: FlowState::new(Measurer::new(Arc::clone(budget), key)),
            stories: FlowState::new(Measurer::new(Arc::clone(budget), key)),
            substitutions: Substitutions::default(),
            style_values: Substitutions::default(),
            computed: Computed::default(),
        }
    }

    /// The measures kept by block (body, stories).
    pub fn kept_blocks(&self) -> (usize, usize) {
        (self.body.cache.len(), self.stories.cache.len())
    }

    /// The headers' and footers' heights of a version.
    pub fn story_heights(&mut self, model: &mut Model, document: &WordDocument) -> StoryTable {
        let cx = StoryContext {
            document,
            faces: self.faces.as_ref(),
            locale: &self.locale,
            tables: self.tables.as_ref(),
        };
        StoryTable::compute(&cx, model, &mut self.stories)
    }

    /// The pages of one pass whose header or footer a `STYLEREF` gives heights of their
    /// own.
    fn page_stories(
        &mut self,
        model: &mut Model,
        layout: &DocumentLayout,
        roles: &[Option<NoteRole>],
        memo: &mut StoryMemo,
    ) -> PageStories {
        let cx = StoryContext {
            document: &layout.document,
            faces: self.faces.as_ref(),
            locale: &self.locale,
            tables: self.tables.as_ref(),
        };
        let job = PageStoryJob { cx: &cx, roles };
        page_stories(&job, model, &mut self.stories, layout, memo)
    }

    /// A version's body flow in its sections, with its paragraphs' list labels (document
    /// order, cells included) and its top-level blocks' parts in the endnotes placed.
    pub fn flow(&mut self, model: &mut Model, document: &WordDocument, frame: &PassFrame<'_>) -> Flow {
        let PassFrame {
            sections,
            labels,
            roles,
            ..
        } = *frame;
        let cx = FlowContext {
            document,
            blocks: &document.body,
            sections,
            labels: Labels::Given(labels),
            faces: self.faces.as_ref(),
            locale: &self.locale,
            tables: self.tables.as_ref(),
            roles,
        };
        FlowMaker::new(&cx, model, &mut self.body).build()
    }

    /// One pass: the flow of `version` and its pages, laid out again from `old` (the same
    /// pass of a layout of the version before) where given.
    fn pass(
        &mut self,
        model: &mut Model,
        version: WordDocument,
        frame: &PassFrame<'_>,
        old: Option<(&Flow, &Pagination, &PageStories)>,
    ) -> DocumentLayout {
        let flow = self.flow(model, &version, frame);
        let mut layout = DocumentLayout {
            flow,
            sections: frame.sections.clone(),
            stories: frame.stories.clone(),
            page_stories: frame.page_stories.clone(),
            pagination: Pagination::default(),
            mode15: crate::input::rules(&version).mode15(),
            hyphen_at_page_end: hyphen_at_page_end(&version),
            document: version,
            fields: FieldValues::new(),
            roles: Arc::from(frame.roles),
            footnote_pages: None,
            passes: 0,
            settled: false,
            earlier: Vec::new(),
            relaid: Vec::new(),
        };
        // Pages laid out with other stories' heights are laid out again whole.
        match old.filter(|(_, _, stories)| *stories == frame.page_stories) {
            Some((old_flow, old_pages, _)) => {
                let found = changes(&old_flow.items, &layout.flow.items);
                let cx = layout.page_context(self.faces.as_ref(), &self.locale);
                let (pages, stats) = repaginate(&cx, old_pages, &found);
                layout.pagination = pages;
                layout.relaid.push(stats);
            }
            None => layout.pagination = paginate_all(&layout, self.faces.as_ref(), &self.locale),
        }
        layout
    }

    /// Lay a version out: every page, laid out again with the values its page-number
    /// fields take until they are the ones it was laid out with (at most
    /// [`FIELD_PASSES`] times; `_Placer.run_full`).
    pub fn lay_out(&mut self, model: &mut Model, document: &WordDocument) -> DocumentLayout {
        self.lay_out_from(model, document, None)
    }

    /// Lay a version out from a layout of the version before: each pass laid out again
    /// from the same pass of `previous`, from the page the first change reaches until the
    /// pages line up again. The layout is the one [`WordLayout::lay_out`] makes.
    pub fn relayout(
        &mut self,
        model: &mut Model,
        document: &WordDocument,
        previous: &DocumentLayout,
    ) -> DocumentLayout {
        self.lay_out_from(model, document, Some(previous))
    }

    fn lay_out_from(
        &mut self,
        model: &mut Model,
        document: &WordDocument,
        previous: Option<&DocumentLayout>,
    ) -> DocumentLayout {
        // The endnotes numbered and placed in the flow; SEQ and REF, which do not depend
        // on the pages, computed once (`_Placer._prepare`).
        let placed = with_endnotes(document);
        let prepared = cross_references(&placed.document, &mut self.computed);
        // STYLEREF in the body: the text of its style before it in the document, else
        // after it -- the document's order, not its pages; computed once, as SEQ is.
        let in_flow = |body: usize| placed.roles.get(body).is_some_and(Option::is_some);
        let mut search = StyleSearch::new(&prepared, &self.locale, &in_flow);
        let prepared = style_references(&prepared, &mut search, &mut self.style_values);
        drop(search);
        let document = &prepared;
        let sections = Sections::of(document);
        let stories = self.story_heights(model, document);
        // A change to the sections or the stories' heights moves every page.
        let previous = previous.filter(|p| p.sections == sections && p.stories == stories);
        let by_page = footnotes_by_page(document);
        let mut version = with_footnotes(document, None);
        // The footnotes' numbers are runs of their own: the fields are found among the
        // runs as numbered.
        let keys = page_fields(&version);
        let labels = document.body_labels();
        let mut overrides = PageStories::new();
        let mut story_memo = StoryMemo::default();
        let mut values = FieldValues::new();
        let mut pages: Option<FootnotePages> = None;
        let mut earlier = Vec::new();
        let mut versions: Vec<WordDocument> = Vec::new();
        let mut relaid = Vec::new();
        loop {
            let frame = PassFrame {
                sections: &sections,
                stories: &stories,
                page_stories: &overrides,
                labels: &labels,
                roles: &placed.roles,
            };
            let old = previous.and_then(|p| p.laid_pass(earlier.len()));
            let mut layout = self.pass(model, version, &frame, old);
            relaid.append(&mut layout.relaid);
            let found = field_values(&layout.document, &layout, &keys);
            let found_pages = by_page.then(|| crate::notes::footnote_pages(&layout));
            // A header's or footer's STYLEREF whose text on a page makes it taller or
            // shorter moves the page's body.
            let heights = self.page_stories(model, &layout, &placed.roles, &mut story_memo);
            let settled = found == values && found_pages == pages && heights == overrides;
            if settled || earlier.len() + 1 == FIELD_PASSES {
                layout.fields = values;
                layout.footnote_pages = pages;
                layout.passes = earlier.len() + 1;
                layout.settled = settled;
                layout.relaid = relaid;
                let mut bodies: Vec<&[ooxml_word_model::model::Block]> = vec![&document.body, &layout.document.body];
                bodies.extend(versions.iter().map(|v| &*v.body));
                self.body.cache.retain_blocks(&bodies);
                self.substitutions.retain_used();
                layout.earlier = earlier;
                return layout;
            }
            versions.push(layout.document.clone());
            earlier.push(PassRecord {
                flow: layout.flow,
                pagination: layout.pagination,
                page_stories: layout.page_stories,
            });
            values = found;
            pages = found_pages;
            overrides = heights;
            let numbered = with_footnotes(document, pages.as_ref());
            version = substitute(&numbered, &values, &mut self.substitutions);
        }
    }
}

/// What every pass of a version is laid out in: its sections, its stories' heights, its
/// paragraphs' list labels, its top-level blocks' parts in the endnotes placed.
#[derive(Debug, Clone, Copy)]
pub struct PassFrame<'f> {
    /// The sections.
    pub sections: &'f Sections,
    /// The headers' and footers' heights.
    pub stories: &'f StoryTable,
    /// The pages whose header or footer has heights of its own.
    pub page_stories: &'f PageStories,
    /// The list labels, document order (cells included).
    pub labels: &'f [Option<String>],
    /// Per top-level block, what of the endnotes placed it is.
    pub roles: &'f [Option<NoteRole>],
}

/// Every page of a laid-out flow, from the start.
pub fn paginate_all(layout: &DocumentLayout, faces: &dyn Faces, locale: &Locale) -> Pagination {
    let cx = layout.page_context(faces, locale);
    Paginator::new(&cx).run(None, &mut |_: &Resume| false).0
}
