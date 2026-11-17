//! Owns: [`WordLayout`], one document's layout engine: what it keeps between versions
//! (measures by block and by content, for the body and for the stories), and laying a
//! version out -- its stories' heights, its flow, its pages -- into a [`DocumentLayout`].
//!
//! Must not: hold more than one document's state, or anything global; edit; draw.

use crate::faces::Faces;
use crate::fields::{cross_references, field_values, page_fields, substitute, Computed, FieldValues, Substitutions};
use crate::flow::{Flow, FlowContext, FlowMaker, FlowState, Labels};
use crate::incremental::{diff, repaginate, RelayoutStats};
use crate::paginate::{HyphenMove, PageContext, Pagination, Paginator, Resume};
use crate::sections::Sections;
use crate::stories::{StoryContext, StoryTable};
use crate::table::TableLayout;
use ooxml_base::{DigestKey, Locale};
use ooxml_derive::CacheBudget;
use ooxml_text::Measurer;
use ooxml_word_model::{Model, WordDocument};
use std::collections::HashMap;
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
}

impl DocumentLayout {
    /// Pass `k`'s flow and pages (the last pass is the layout's own).
    pub fn pass(&self, k: usize) -> Option<(&Flow, &Pagination)> {
        match self.earlier.get(k) {
            Some(p) => Some((&p.flow, &p.pagination)),
            None if k == self.earlier.len() => Some((&self.flow, &self.pagination)),
            None => None,
        }
    }
}

/// At most this many layouts of the body until its page-number fields settle.
pub const FIELD_PASSES: usize = 4;

impl DocumentLayout {
    /// The context its pages were laid out in.
    pub fn page_context(&self) -> PageContext<'_> {
        PageContext {
            items: &self.flow.items,
            sections: &self.sections,
            mode15: self.mode15,
            hyphen_at_page_end: self.hyphen_at_page_end,
            stories: &self.stories,
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

    /// A version's body flow in its sections, with its paragraphs' list labels (document
    /// order, cells included); `note_runs`: per top-level block, the runs that draw a
    /// note's number.
    pub fn flow(
        &mut self,
        model: &mut Model,
        document: &WordDocument,
        frame: (&Sections, &[Option<String>]),
        note_runs: &HashMap<usize, Vec<bool>>,
    ) -> Flow {
        let (sections, labels) = frame;
        let cx = FlowContext {
            document,
            blocks: &document.body,
            sections,
            labels: Labels::Given(labels),
            faces: self.faces.as_ref(),
            locale: &self.locale,
            tables: self.tables.as_ref(),
            note_runs,
        };
        FlowMaker::new(&cx, model, &mut self.body).build()
    }

    /// One pass: the flow of `version` and its pages, laid out again from `old` (the same
    /// pass of a layout of the version before) where given.
    fn pass(
        &mut self,
        model: &mut Model,
        version: WordDocument,
        frame: (&Sections, &StoryTable, &[Option<String>]),
        old: Option<(&Flow, &Pagination)>,
    ) -> DocumentLayout {
        let (sections, stories, labels) = frame;
        let flow = self.flow(model, &version, (sections, labels), &HashMap::new());
        let mut layout = DocumentLayout {
            flow,
            sections: sections.clone(),
            stories: stories.clone(),
            pagination: Pagination::default(),
            mode15: crate::input::rules(&version).mode15(),
            hyphen_at_page_end: hyphen_at_page_end(&version),
            document: version,
            fields: FieldValues::new(),
            passes: 0,
            settled: false,
            earlier: Vec::new(),
            relaid: Vec::new(),
        };
        match old {
            Some((old_flow, old_pages)) => {
                let change = diff(&old_flow.items, &layout.flow.items);
                let (pages, stats) = repaginate(&layout.page_context(), old_pages, change);
                layout.pagination = pages;
                layout.relaid.push(stats);
            }
            None => layout.pagination = paginate_all(&layout),
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
        // SEQ and REF do not depend on the pages: computed once (`_Placer._prepare`).
        let prepared = cross_references(document, &mut self.computed);
        let document = &prepared;
        let sections = Sections::of(document);
        let stories = self.story_heights(model, document);
        // A change to the sections or the stories' heights moves every page.
        let previous = previous.filter(|p| p.sections == sections && p.stories == stories);
        let keys = page_fields(document);
        let labels = document.body_labels();
        let mut values = FieldValues::new();
        let mut version = document.clone();
        let mut earlier = Vec::new();
        let mut versions: Vec<WordDocument> = Vec::new();
        let mut relaid = Vec::new();
        loop {
            let old = previous.and_then(|p| p.pass(earlier.len()));
            let mut layout = self.pass(model, version, (&sections, &stories, &labels), old);
            relaid.append(&mut layout.relaid);
            let found = field_values(&layout.document, &layout, &keys);
            let settled = found == values;
            if settled || earlier.len() + 1 == FIELD_PASSES {
                layout.fields = values;
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
            });
            values = found;
            version = substitute(document, &values, &mut self.substitutions);
        }
    }
}

/// Every page of a laid-out flow, from the start.
pub fn paginate_all(layout: &DocumentLayout) -> Pagination {
    let cx = layout.page_context();
    Paginator::new(&cx).run(None, &mut |_: &Resume| false).0
}
