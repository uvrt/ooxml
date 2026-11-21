//! Owns: one document's page engine ([`PageEngine`]): the versions it is given (a
//! package's bytes, of which only the parts whose stored bytes changed are read again, or
//! a version's touched parts), the Word model's and the layout's memos kept between
//! versions, the last version laid out -- which the next is laid out from, and which is the
//! answer again for a version with the same parts (a caller asking for the version it has,
//! as a field update's first pass does) -- and painting a version's pages.
//!
//! A version is laid out only where the layout lays it out as docx2svg does
//! ([`crate::support`]); otherwise the answer is a [`Fallback`], and the engine keeps
//! nothing of it. A change of the document's fonts (its font table or embedded faces)
//! starts a new generation: new memos, nothing laid out from the old.
//!
//! What it keeps is bounded (lane L4b, design section 6, Memory): its memos are swept once
//! a version is laid out (what no version in use holds goes), and what it painted is kept
//! by page within its budget's bounded account, the least recently used let go first
//! ([`EngineBudgets`]).
//!
//! Must not: be used by two callers at once (its owner locks it), or keep more than the
//! last version laid out.

mod parts;

use crate::causes::{context_cause, part_reason, whole_cause, RelaidCause};
use crate::digests::Lines;
use crate::error::{Fallback, PagesError};
use crate::fonts::{DocumentFonts, FontSetup};
use crate::stats::EngineStats;
use crate::support;
use crate::text_boxes::{text_boxes, TextBoxJob};
use crate::version::{LaidVersion, PageView};
use ooxml_base::{Cancel, DigestKey, Limits, Locale};
use ooxml_derive::{CacheBudget, Memo, Weigh};
use ooxml_opc::Package;
use ooxml_word_layout::paint::svg::{page_svg, Drawings, Pictures, SvgContext};
use ooxml_word_layout::paint::{paint_page, GlyphSize, PagePaint, PaintContext};
use ooxml_word_layout::WordLayout;
use ooxml_word_model::Model;
use ooxml_zip::Archive;
use parts::{carried_facts, new_layout, pictures_of, say, version_ids, SayWith};
use std::sync::Arc;
use std::time::Instant;

/// The key the engine digests content under.
const KEY: DigestKey = DigestKey::new(0x6f6f_786d_6c2d_7061, 0x6765_732d_4c34_0001);

/// What a generation holds: the fonts, the model, the layout's memos, the last version.
struct Generation {
    number: u64,
    fonts: DocumentFonts,
    model: Model,
    layout: WordLayout,
    last: Option<Arc<LaidVersion>>,
    /// The package `last` was laid out from: a version with the same parts is `last`.
    laid_from: Option<Package>,
    /// What each page of the last version said (its warnings), for the pages the next
    /// version keeps.
    said: crate::page_said::PageMemo,
    /// The main part's root and its ids as the last version read them, which the next
    /// version's ids are found from (only what changed walked again).
    ids: Option<(Arc<ooxml_xml::Element>, Arc<ooxml_word_model::ids::PartIds>)>,
    /// What each page of the version summarised last digested to.
    summaries: crate::summary::SummaryMemo,
}

impl Generation {
    /// The last version laid out where `package` has its parts (the same version asked
    /// again).
    fn last_for(&self, package: &Package) -> Option<Arc<LaidVersion>> {
        let from = self.laid_from.as_ref()?;
        from.same_parts(package).then(|| self.last.clone()).flatten()
    }

    /// `document` laid out, from `previous` where given, stopped where `cancel` says so.
    fn lay_out(
        &mut self,
        document: &ooxml_word_model::WordDocument,
        previous: Option<&LaidVersion>,
        cancel: Option<&dyn Cancel>,
    ) -> Result<ooxml_word_layout::DocumentLayout, ooxml_base::BaseError> {
        let (layout, model) = (&mut self.layout, &mut self.model);
        match (cancel, previous) {
            (Some(cancel), _) => layout.lay_out_until(model, document, previous.map(|p| &p.layout), cancel),
            (None, Some(previous)) => Ok(layout.relayout(model, document, &previous.layout)),
            (None, None) => Ok(layout.lay_out(model, document)),
        }
    }

    /// A layout stopped: the version laid out before (`last`, laid out from `from`) is
    /// still the one the next is laid out from. Where a part beside the main one changed
    /// (`others_changed`), what its pages said goes: the next version, read against the
    /// stopped one's parts, would not see that change.
    fn stopped(&mut self, last: Option<Arc<LaidVersion>>, from: Option<Package>, others_changed: bool) {
        self.last = last;
        self.laid_from = from;
        if others_changed {
            self.said = crate::page_said::PageMemo::default();
            self.summaries = crate::summary::SummaryMemo::default();
        }
    }
}

/// A line's paragraph path, its story (a header's or footer's: its kind and part), its
/// paint digested.
pub type LineDigest = (
    String,
    Option<(&'static str, ooxml_word_model::value::Text)>,
    ooxml_base::Digest,
);

/// What a page paints, digested ([`PageEngine::page_digests`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageDigests {
    /// Its frame, rules, pictures and drawings.
    pub rest: ooxml_base::Digest,
    /// Each line: its paragraph's path, its story (a header's or footer's: its kind and
    /// part), its paint.
    pub lines: Vec<LineDigest>,
}

/// The package a version was read from, kept to read the next one against.
struct Source {
    archive: Arc<Archive>,
    package: Package,
}

/// Where an engine's memory is counted: what the version in use needs (its memos, never
/// refused) and what can be made again (its pages' paint, within a limit).
#[derive(Clone)]
pub struct EngineBudgets {
    /// The model's and the layout's memos.
    pub held: Arc<dyn CacheBudget>,
    /// The pages painted, kept for the next reader while the budget has room.
    pub paint: Arc<dyn CacheBudget>,
}

impl std::fmt::Debug for EngineBudgets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineBudgets")
            .field("held", &self.held.used())
            .field("paint", &self.paint.used())
            .finish()
    }
}

/// A page painted, as the paint memo keeps it.
#[derive(Clone)]
struct Painted(Arc<PagePaint>);

impl Weigh for Painted {
    fn weight(&self) -> usize {
        crate::weight::paint_weight(&self.0)
    }
}

/// One document's page engine.
pub struct PageEngine {
    setup: Arc<FontSetup>,
    locale: Locale,
    numbers: ooxml_numfmt::Locale,
    decimal: char,
    glyph_size: GlyphSize,
    budgets: EngineBudgets,
    /// Pages painted, by version (its serial) and page.
    painted: Memo<(u64, usize), Painted>,
    /// The serial the next version laid out takes.
    serial: u64,
    limits: Limits,
    generation: Option<Generation>,
    source: Option<Source>,
    generations: u64,
    stats: EngineStats,
    /// The parts the version being laid out changed from the last one given.
    changed: Vec<String>,
}

impl std::fmt::Debug for PageEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageEngine")
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

/// How an engine draws and lays out, beyond its fonts.
#[derive(Debug, Clone)]
pub struct EngineOptions {
    /// The locale (hyphenation, the layout's language rules).
    pub locale: Locale,
    /// How a chart's numbers and dates are written: the caller's locale's separators, date
    /// order and language (docx2svg's `ConvertOptions.locale`), en-US unless given.
    pub numbers: ooxml_numfmt::Locale,
    /// The decimal separator a decimal tab lines up.
    pub decimal: char,
    /// What glyph outlines are scaled to in the SVG.
    pub glyph_size: GlyphSize,
    /// The package's limits.
    pub limits: Limits,
}

impl Default for EngineOptions {
    fn default() -> EngineOptions {
        EngineOptions {
            locale: Locale::default(),
            numbers: ooxml_numfmt::Locale::default(),
            decimal: '.',
            glyph_size: GlyphSize::Device,
            limits: Limits::default(),
        }
    }
}

impl PageEngine {
    /// An engine laying out with `setup`'s fonts, its memory counted in `budgets`.
    pub fn new(setup: Arc<FontSetup>, options: EngineOptions, budgets: EngineBudgets) -> PageEngine {
        PageEngine {
            setup,
            locale: options.locale,
            numbers: options.numbers,
            decimal: options.decimal,
            glyph_size: options.glyph_size,
            painted: Memo::new(Arc::clone(&budgets.paint)),
            serial: 0,
            budgets,
            limits: options.limits,
            generation: None,
            source: None,
            generations: 0,
            stats: EngineStats::default(),
            changed: Vec::new(),
        }
    }

    /// What it did so far.
    pub fn stats(&self) -> EngineStats {
        self.stats
    }

    /// The last version laid out, if any.
    pub fn last(&self) -> Option<&Arc<LaidVersion>> {
        self.generation.as_ref().and_then(|g| g.last.as_ref())
    }

    /// Lay out the version whose package is `bytes`: only the parts whose stored bytes
    /// differ from the last version given are read again.
    ///
    /// # Errors
    /// The package or its Word parts do not open.
    pub fn lay_out_bytes(&mut self, bytes: Arc<[u8]>) -> Result<Result<Arc<LaidVersion>, Fallback>, PagesError> {
        let archive = Arc::new(Archive::read(bytes, &self.limits.zip).map_err(ooxml_opc::OpcError::from)?);
        self.changed.clear();
        let package = match self.source.take() {
            Some(before) => self.next_package(&before, &archive)?,
            None => Package::open(Arc::clone(archive.bytes()), &self.limits)?,
        };
        self.source = Some(Source {
            archive,
            package: package.clone(),
        });
        self.lay_out(&package)
    }

    /// Lay out the version of the last one given with `parts` put in place (bytes) or
    /// removed (`None`): the bridge's touched parts.
    ///
    /// # Errors
    /// No version was given before, or the parts do not open.
    pub fn update(
        &mut self,
        parts: &[(String, Option<Arc<[u8]>>)],
    ) -> Result<Result<Arc<LaidVersion>, Fallback>, PagesError> {
        let Some(source) = self.source.take() else {
            return Err(PagesError::Package(ooxml_opc::OpcError::NoPart(
                "no version to update: lay one out first".into(),
            )));
        };
        let mut package = source.package.clone();
        self.changed = parts.iter().map(|(name, _)| name.clone()).collect();
        for (name, bytes) in parts {
            package = match bytes {
                Some(bytes) => self.put(&package, name, bytes)?,
                None if package.has_part(name) => package.without(name)?,
                None => package,
            };
        }
        self.source = Some(Source {
            archive: source.archive,
            package: package.clone(),
        });
        self.lay_out(&package)
    }

    /// `package` with `name` as `bytes`: a tree, parsed once, where the part is XML.
    fn put(&self, package: &Package, name: &str, bytes: &Arc<[u8]>) -> Result<Package, PagesError> {
        let xml = name.ends_with(".xml") || name.ends_with(".rels");
        Ok(if xml {
            let tree = ooxml_xml::parse(bytes, &self.limits.xml).map_err(|error| ooxml_opc::OpcError::Xml {
                part: name.to_owned(),
                error,
            })?;
            package.with_new_tree(name, tree)
        } else {
            package.with_bytes(name, Arc::clone(bytes))
        })
    }

    /// The version in `archive`, from the one in `before`: each entry whose stored bytes
    /// are the same is the part as it was (its tree shared), the rest read again.
    fn next_package(&mut self, before: &Source, archive: &Archive) -> Result<Package, PagesError> {
        let old: std::collections::HashMap<&str, &ooxml_zip::Entry> = before
            .archive
            .entries()
            .iter()
            .map(|e| (e.meta().name.as_str(), e))
            .collect();
        let mut package = before.package.clone();
        let mut seen = std::collections::HashSet::new();
        for entry in archive.entries() {
            let meta = entry.meta();
            if meta.is_dir() {
                continue;
            }
            seen.insert(meta.name.as_str());
            let same = old.get(meta.name.as_str()).is_some_and(|o| {
                let m = o.meta();
                m.crc32 == meta.crc32
                    && m.size == meta.size
                    && m.method == meta.method
                    && before.archive.stored_data(o) == archive.stored_data(entry)
            });
            if same {
                self.stats.parts_kept += 1;
                continue;
            }
            self.stats.parts_read += 1;
            self.changed.push(meta.name.clone());
            let bytes: Arc<[u8]> = archive.read_entry(entry).map_err(ooxml_opc::OpcError::from)?.into();
            package = self.put(&package, &meta.name, &bytes)?;
        }
        for name in old.keys().filter(|n| !seen.contains(*n) && !n.ends_with('/')) {
            if package.has_part(name) {
                self.changed.push((*name).to_owned());
                package = package.without(name)?;
            }
        }
        Ok(package)
    }

    /// Lay out the version `package`, from the last version laid out where the fonts are
    /// the same.
    ///
    /// # Errors
    /// Its Word parts do not open.
    pub fn lay_out(&mut self, package: &Package) -> Result<Result<Arc<LaidVersion>, Fallback>, PagesError> {
        self.lay_out_within(package, None)
    }

    /// [`PageEngine::lay_out`], stopped where `cancel` says so (asked before each top-level
    /// block of the body's flow and before each page). The version laid out last, the same
    /// parts asked again, is answered whatever `cancel` says: nothing is laid out.
    ///
    /// # Errors
    /// Its Word parts do not open; [`PagesError::Stopped`] where `cancel` stopped it: the
    /// engine then lays the next version out from the one it laid out before.
    pub fn lay_out_until(
        &mut self,
        package: &Package,
        cancel: &dyn Cancel,
    ) -> Result<Result<Arc<LaidVersion>, Fallback>, PagesError> {
        self.lay_out_within(package, Some(cancel))
    }

    fn lay_out_within(
        &mut self,
        package: &Package,
        cancel: Option<&dyn Cancel>,
    ) -> Result<Result<Arc<LaidVersion>, Fallback>, PagesError> {
        self.changed.sort();
        let fonts = self.setup.document(package);
        let (generation, mut cause) = self.generation_for(fonts);
        let generation = self.generation.insert(generation);
        if let Some(last) = generation.last_for(package) {
            self.stats.kept += 1;
            return Ok(Ok(last));
        }
        let started = Instant::now();
        let document = generation.model.open(package)?;
        self.stats.open_time += started.elapsed();
        if generation.model.stats().context_changed && generation.last.is_some() {
            // The styles, numbering, settings or theme changed: what was measured under the
            // old ones is not this version's (the measures are keyed by content), so it is
            // laid out whole, with fresh memos.
            generation.last = None;
            generation.layout = new_layout(&generation.fonts, &self.locale, &self.budgets.held);
            cause = Some(context_cause(&self.changed));
        }
        if let Some(why) = support::unsupported(&document) {
            generation.last = None;
            self.stats.fallbacks += 1;
            return Ok(Err(why));
        }
        let started = Instant::now();
        let previous = generation.last.take();
        let laid_from = generation.laid_from.take();
        let Ok(laid) = generation.lay_out(&document, previous.as_deref(), cancel) else {
            self.stats.layout_time += started.elapsed();
            self.stats.stopped += 1;
            let others_changed = self.changed.iter().any(|name| *name != *document.main_part);
            generation.stopped(previous, laid_from, others_changed);
            return Err(PagesError::Stopped);
        };
        if let (Some(previous), true) = (&previous, laid.relaid.is_empty()) {
            cause = whole_cause(&previous.layout, &laid, &self.changed);
        }
        self.stats.layout_time += started.elapsed();
        if let Some(why) = support::stopped(&laid) {
            self.stats.fallbacks += 1;
            return Ok(Err(why));
        }
        let relaid = !laid.relaid.is_empty();
        self.stats.count_laid(relaid);
        let (pictures, drawings) = pictures_of(package, &document, &generation.fonts, &self.numbers);
        let others_changed = self.changed.iter().any(|name| *name != *document.main_part);
        let with = SayWith {
            locale: &self.locale,
            glyph_size: self.glyph_size,
            decimal: self.decimal,
            others_changed,
        };
        let facts = carried_facts(previous.as_deref(), &laid);
        let said = say(generation, &laid, drawings.as_ref(), &with, &facts);
        self.stats.pages_said = generation.said.stats();
        let ids = version_ids(generation, package, &document.main_part);
        self.serial += 1;
        let version = Arc::new(LaidVersion {
            drawings,
            said,
            relaid_cause: cause,
            layout: laid,
            facts,
            pictures,
            glyphs: Arc::clone(&generation.fonts.glyphs),
            names: Arc::clone(&generation.fonts.names),
            relaid,
            generation: generation.number,
            serial: self.serial,
            ids,
            summary: std::sync::OnceLock::new(),
        });
        // The version before is no longer the one laid out from: what only it and the
        // memos held goes now, not with every version kept.
        drop(previous);
        generation.last = Some(Arc::clone(&version));
        generation.laid_from = Some(package.clone());
        let swept = generation.model.sweep() + generation.layout.sweep();
        self.stats.swept += swept as u64;
        Ok(Ok(version))
    }

    /// Why a version was laid out whole, from the first changed part of `reason`'s kind.
    fn part_cause(&self, reason: &'static str) -> RelaidCause {
        RelaidCause {
            reason,
            part: self.changed.iter().find(|n| part_reason(n) == Some(reason)).cloned(),
            ..RelaidCause::default()
        }
    }

    /// The generation `fonts` lay out in: the current one where its fonts are the same,
    /// else a new one, with the cause of laying out whole where the old had a version.
    fn generation_for(&mut self, fonts: DocumentFonts) -> (Generation, Option<RelaidCause>) {
        match self.generation.take() {
            Some(g) if g.fonts.epoch == fonts.epoch => (g, None),
            Some(g) => {
                let cause = g.last.is_some().then(|| self.part_cause("fonts"));
                (self.new_generation(fonts), cause)
            }
            None => (self.new_generation(fonts), None),
        }
    }

    fn new_generation(&mut self, fonts: DocumentFonts) -> Generation {
        self.generations += 1;
        Generation {
            number: self.generations,
            model: Model::new(Arc::clone(&self.budgets.held), KEY),
            layout: new_layout(&fonts, &self.locale, &self.budgets.held),
            fonts,
            last: None,
            laid_from: None,
            said: crate::page_said::PageMemo::default(),
            ids: None,
            summaries: crate::summary::SummaryMemo::default(),
        }
    }

    /// Let go of the last version laid out and the memos: the next version is laid out
    /// whole.
    pub fn forget(&mut self) {
        self.generation = None;
        self.source = None;
        self.painted.clear();
    }

    /// What its memos hold now (bytes counted on its `held` budget) and what its pages
    /// painted hold (on its `paint` budget).
    pub fn memory(&self) -> (usize, usize) {
        (self.budgets.held.used(), self.painted.stats().bytes)
    }

    /// The locale it lays out in.
    pub fn locale(&self) -> &Locale {
        &self.locale
    }

    /// Page `page` of `version` (0-based, blank pages counted) painted: `None` past its
    /// last page, or where `version` is not of this engine's generation. Kept for the
    /// next reader while the paint budget has room.
    pub fn paint(&mut self, version: &LaidVersion, page: usize) -> Option<Arc<PagePaint>> {
        self.painted_page(version, page, true)
    }

    /// Page `page` of `version` painted, taken from the pages kept painted where it is one;
    /// kept for the next reader where `keep` (a reader's page), not where it is painted to
    /// be read once (a scan of every page: [`PageEngine::page_digests`]).
    pub(crate) fn painted_page(&mut self, version: &LaidVersion, page: usize, keep: bool) -> Option<Arc<PagePaint>> {
        let key = (version.serial, page);
        if let Some(Painted(found)) = self.painted.get(&key) {
            self.stats.paint_hits += 1;
            return Some(found);
        }
        let generation = self.generation.as_mut().filter(|g| g.number == version.generation)?;
        let parts = generation
            .layout
            .page_parts(&mut generation.model, &version.layout, page);
        let cx = PaintContext {
            faces: version.glyphs.as_ref(),
            names: version.names.as_ref(),
            glyph_size: self.glyph_size,
            decimal: self.decimal,
        };
        let mut paint = paint_page(&version.layout, &mut generation.model, page, &parts, &cx)?;
        if let Some(drawings) = &version.drawings {
            let job = TextBoxJob {
                layout: &mut generation.layout,
                model: &mut generation.model,
                laid: &version.layout,
                cx: &cx,
            };
            paint.text_boxes = text_boxes(job, drawings, &paint, page);
            let unpainted: Vec<String> = paint.text_boxes.iter().flat_map(|t| t.unpainted.clone()).collect();
            paint.unpainted.extend(unpainted);
        }
        let paint = Arc::new(paint);
        self.stats.pages_painted += 1;
        if keep {
            self.painted.insert(key, Painted(Arc::clone(&paint)));
        }
        Some(paint)
    }

    /// The header's and footer's lines page `page` of `version` draws, as a check reads
    /// them: laid out with the page's field values, not painted. `None` for a version of
    /// another generation or a page past the last.
    pub fn story_lines(
        &mut self,
        version: &LaidVersion,
        page: usize,
    ) -> Option<Vec<ooxml_word_layout::facts::StoryLineFact>> {
        let generation = self.generation.as_mut().filter(|g| g.number == version.generation)?;
        if page >= version.page_count() {
            return None;
        }
        let parts = generation
            .layout
            .page_parts(&mut generation.model, &version.layout, page);
        Some(ooxml_word_layout::facts::story_line_facts(&parts))
    }

    /// What page `page` of `version` paints, digested: what besides its lines (its frame,
    /// rules, pictures and drawings), and each line by its paragraph's path and story --
    /// what tells two versions' pages apart without a reader reading them.
    pub fn page_digests(&mut self, version: &LaidVersion, page: usize) -> Option<PageDigests> {
        let paint = self.painted_page(version, page, false)?;
        Some(crate::digests::digests(&paint, KEY, Lines::Page))
    }

    /// `version`'s pages compared as the layouts themselves are (docx-agent's
    /// `DocumentLayout.compare`, over `Page.text_lines`): its summary with every line of
    /// text a page draws, its text boxes' too; made whole, never kept.
    pub fn contents(&mut self, version: &LaidVersion) -> Option<crate::summary::Summary> {
        let mut pages = Vec::with_capacity(version.layout.page_count());
        for page in 0..version.layout.page_count() {
            let paint = self.painted_page(version, page, false)?;
            let digests = crate::digests::digests(&paint, KEY, Lines::WithTextBoxes);
            pages.push(Arc::new(crate::summary::page_summary(digests, version)));
        }
        let known = pages.len();
        Some(crate::summary::Summary { pages, known })
    }

    /// Page `page` of `version`'s lines' text, as a check reads it ([`crate::line_text`]):
    /// from its paint, taken from the pages kept painted where it is one, not kept itself (a
    /// check reads every page's text once: its reader keeps the text, not the paint).
    pub fn line_texts(&mut self, version: &LaidVersion, page: usize) -> Option<Vec<crate::line_text::LineText>> {
        let paint = self.painted_page(version, page, false)?;
        Some(paint.lines.iter().map(crate::line_text::LineText::of).collect())
    }

    /// `version`'s summary (docx-agent's `LayoutSummary`): each page's taken from the
    /// version summarised before where the relayout kept that page with the same items and
    /// context, the rest painted and digested; `None` where `version` is not of this
    /// engine's generation.
    pub fn summary(&mut self, version: &LaidVersion) -> Option<Arc<crate::summary::Summary>> {
        if let Some(found) = version.summary.get() {
            return Some(Arc::clone(found));
        }
        if self.generation.as_ref().is_none_or(|g| g.number != version.generation) {
            return None;
        }
        let layout = &version.layout;
        let context = crate::summary::context_of(layout, KEY);
        let mut pages = Vec::with_capacity(layout.page_count());
        let mut kept = Vec::new();
        for page in 0..layout.page_count() {
            let key = crate::page_said::page_key(layout, page);
            let found = key
                .as_ref()
                .and_then(|k| self.generation.as_mut().and_then(|g| g.summaries.take(k, context)));
            let summary = match found {
                Some(found) => found,
                None => Arc::new(crate::summary::page_summary(self.page_digests(version, page)?, version)),
            };
            if let Some(key) = key {
                kept.push((key, Arc::clone(&summary)));
            }
            pages.push(summary);
        }
        if let Some(generation) = self.generation.as_mut() {
            generation.summaries.keep(kept, context);
        }
        let made = Arc::new(crate::summary::Summary {
            known: pages.len(),
            pages,
        });
        Some(Arc::clone(version.summary.get_or_init(|| made)))
    }

    /// Page `page` of `version` as a reader sees it.
    pub fn page(&mut self, version: &LaidVersion, page: usize) -> Option<PageView> {
        let paint = self.paint(version, page)?;
        let blocks = version.page_facts(page).map(|p| p.blocks.clone()).unwrap_or_default();
        let paint = Arc::unwrap_or_clone(paint);
        Some(PageView {
            page,
            info: paint.geometry.info,
            frame: paint.geometry.frame,
            lines: paint.lines,
            rules: paint.rules,
            pictures: paint.pictures,
            drawing_faces: paint
                .drawings
                .iter()
                .map(|slot| version.drawings.as_ref().map(|d| d.faces(slot)).unwrap_or_default())
                .collect(),
            drawings: paint.drawings,
            blocks,
            unpainted: paint.unpainted,
        })
    }

    /// Page `page` of `version` as SVG, byte for byte docx2svg's where nothing is listed
    /// beside it; its drawings painted by the version's painter. Beside it, what is not
    /// drawn as docx2svg draws it (empty where nothing): the page's own reasons.
    pub fn page_svg(&mut self, version: &LaidVersion, page: usize) -> Option<(String, Vec<String>)> {
        let paint = self.paint(version, page)?;
        let mut missing: Vec<String> = paint.unpainted.iter().map(|_| support::UNPAINTED.to_owned()).collect();
        let none = ooxml_word_layout::paint::svg::NoPictures;
        let no_drawings = ooxml_word_layout::paint::svg::NoDrawings;
        let drawings: &dyn Drawings = match &version.drawings {
            Some(d) => d,
            None => {
                if !paint.drawings.is_empty() {
                    missing.push(support::DRAWINGS.to_owned());
                }
                &no_drawings
            }
        };
        let cx = SvgContext {
            drawings,
            names: version.names.as_ref(),
            pictures: version
                .pictures
                .as_ref()
                .map_or(&none as &dyn Pictures, |p| p as &dyn Pictures),
            glyph_size: self.glyph_size,
        };
        Some((page_svg(&paint, &cx), missing))
    }
}
