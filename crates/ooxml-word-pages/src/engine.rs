//! Owns: one document's page engine ([`PageEngine`]): the versions it is given (a
//! package's bytes, of which only the parts whose stored bytes changed are read again, or
//! a version's touched parts), the Word model's and the layout's memos kept between
//! versions, the last version laid out -- which the next is laid out from -- and painting
//! a version's pages.
//!
//! A version is laid out only where the layout lays it out as docx2svg does
//! ([`crate::support`]); otherwise the answer is a [`Fallback`], and the engine keeps
//! nothing of it. A change of the document's fonts (its font table or embedded faces)
//! starts a new generation: new memos, nothing laid out from the old.
//!
//! Must not: be used by two callers at once (its owner locks it), or keep more than the
//! last version laid out.

use crate::error::{Fallback, PagesError};
use crate::fonts::{DocumentFonts, FontSetup};
use crate::pictures::PackagePictures;
use crate::support;
use crate::version::{LaidVersion, PageView};
use ooxml_base::{DigestKey, Limits, Locale};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_word_layout::paint::drawings::DrawingSlot;
use ooxml_word_layout::paint::svg::{page_svg, Drawings, Pictures, SvgContext};
use ooxml_word_layout::paint::{paint_page, GlyphSize, PagePaint, PaintContext};
use ooxml_word_layout::tables::WordTables;
use ooxml_word_layout::WordLayout;
use ooxml_word_model::Model;
use ooxml_zip::Archive;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The key the engine digests content under.
const KEY: DigestKey = DigestKey::new(0x6f6f_786d_6c2d_7061, 0x6765_732d_4c34_0001);

/// What an engine did, since it was made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineStats {
    /// Versions laid out whole.
    pub full: u64,
    /// Versions laid out again from the version before.
    pub relaid: u64,
    /// Versions answered with a fallback.
    pub fallbacks: u64,
    /// Parts read again (their stored bytes changed), over every version.
    pub parts_read: u64,
    /// Parts taken as they were.
    pub parts_kept: u64,
    /// Pages painted.
    pub pages_painted: u64,
    /// Time spent opening versions (the model).
    pub open_time: Duration,
    /// Time spent laying out.
    pub layout_time: Duration,
}

/// What a generation holds: the fonts, the model, the layout's memos, the last version.
struct Generation {
    number: u64,
    fonts: DocumentFonts,
    model: Model,
    layout: WordLayout,
    last: Option<Arc<LaidVersion>>,
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

/// One document's page engine.
pub struct PageEngine {
    setup: Arc<FontSetup>,
    locale: Locale,
    decimal: char,
    glyph_size: GlyphSize,
    budget: Arc<dyn CacheBudget>,
    limits: Limits,
    generation: Option<Generation>,
    source: Option<Source>,
    generations: u64,
    stats: EngineStats,
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
            decimal: '.',
            glyph_size: GlyphSize::Device,
            limits: Limits::default(),
        }
    }
}

impl PageEngine {
    /// An engine laying out with `setup`'s fonts, its memos within `budget`.
    pub fn new(setup: Arc<FontSetup>, options: EngineOptions, budget: Arc<dyn CacheBudget>) -> PageEngine {
        PageEngine {
            setup,
            locale: options.locale,
            decimal: options.decimal,
            glyph_size: options.glyph_size,
            budget,
            limits: options.limits,
            generation: None,
            source: None,
            generations: 0,
            stats: EngineStats::default(),
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
            let bytes: Arc<[u8]> = archive.read_entry(entry).map_err(ooxml_opc::OpcError::from)?.into();
            package = self.put(&package, &meta.name, &bytes)?;
        }
        for name in old.keys().filter(|n| !seen.contains(*n) && !n.ends_with('/')) {
            if package.has_part(name) {
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
        let fonts = self.setup.document(package);
        let generation = match self.generation.take() {
            Some(g) if g.fonts.epoch == fonts.epoch => g,
            _ => self.new_generation(fonts),
        };
        let generation = self.generation.insert(generation);
        let started = Instant::now();
        let document = generation.model.open(package)?;
        self.stats.open_time += started.elapsed();
        if generation.model.stats().context_changed && generation.last.is_some() {
            // The styles, numbering, settings or theme changed: what was measured under the
            // old ones is not this version's (the measures are keyed by content), so it is
            // laid out whole, with fresh memos.
            generation.last = None;
            generation.layout = new_layout(&generation.fonts, &self.locale, &self.budget);
        }
        if let Some(why) = support::unsupported(&document) {
            generation.last = None;
            self.stats.fallbacks += 1;
            return Ok(Err(why));
        }
        let started = Instant::now();
        let laid = match generation.last.take() {
            Some(previous) => generation
                .layout
                .relayout(&mut generation.model, &document, &previous.layout),
            None => generation.layout.lay_out(&mut generation.model, &document),
        };
        self.stats.layout_time += started.elapsed();
        if let Some(why) = support::stopped(&laid) {
            self.stats.fallbacks += 1;
            return Ok(Err(why));
        }
        let relaid = !laid.relaid.is_empty();
        if relaid {
            self.stats.relaid += 1;
        } else {
            self.stats.full += 1;
        }
        let version = Arc::new(LaidVersion {
            layout: laid,
            facts: std::sync::OnceLock::new(),
            pictures: PackagePictures::new(package.clone()),
            glyphs: Arc::clone(&generation.fonts.glyphs),
            names: Arc::clone(&generation.fonts.names),
            relaid,
            generation: generation.number,
        });
        generation.last = Some(Arc::clone(&version));
        Ok(Ok(version))
    }

    fn new_generation(&mut self, fonts: DocumentFonts) -> Generation {
        self.generations += 1;
        Generation {
            number: self.generations,
            model: Model::new(Arc::clone(&self.budget), KEY),
            layout: new_layout(&fonts, &self.locale, &self.budget),
            fonts,
            last: None,
        }
    }

    /// Let go of the last version laid out and the memos: the next version is laid out
    /// whole.
    pub fn forget(&mut self) {
        self.generation = None;
        self.source = None;
    }

    /// Page `page` of `version` (0-based, blank pages counted) painted: `None` past its
    /// last page, or where `version` is not of this engine's generation.
    pub fn paint(&mut self, version: &LaidVersion, page: usize) -> Option<PagePaint> {
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
        let paint = paint_page(&version.layout, &mut generation.model, page, &parts, &cx)?;
        self.stats.pages_painted += 1;
        Some(paint)
    }

    /// What page `page` of `version` paints, digested: what besides its lines (its frame,
    /// rules, pictures and drawings), and each line by its paragraph's path and story --
    /// what tells two versions' pages apart without a reader reading them.
    pub fn page_digests(&mut self, version: &LaidVersion, page: usize) -> Option<PageDigests> {
        let paint = self.paint(version, page)?;
        let digest = |value: &dyn std::fmt::Debug| ooxml_base::Digest::of(KEY, format!("{value:?}").as_bytes());
        // What docx-agent's summary digests (`_page_digest`, `_line_digest`): what is drawn
        // and where, never the paths it is drawn for (an edit before a page moves them).
        let rules: Vec<_> = paint
            .rules
            .iter()
            .map(|r| (r.kind, r.x, r.y, r.width, r.height, &r.color, &r.style))
            .collect();
        let pictures: Vec<_> = paint
            .pictures
            .iter()
            .map(|p| (&p.relationship, &p.part, p.x, p.y, p.width, p.height))
            .collect();
        let drawings: Vec<_> = paint
            .drawings
            .iter()
            .map(|d| (d.x, d.y, d.width, d.height, d.layer, d.story.as_ref().map(|s| s.0)))
            .collect();
        let frame = (paint.geometry.frame.width_px, paint.geometry.frame.height_px);
        let rest = digest(&(frame, rules, pictures, drawings));
        let lines = paint
            .lines
            .iter()
            .map(|line| {
                let spans: Vec<_> = line
                    .spans
                    .iter()
                    .map(|s| {
                        (
                            &s.chars,
                            &s.xs,
                            s.y,
                            s.end,
                            &s.face,
                            s.bold,
                            s.italic,
                            s.half_points,
                            &s.color,
                            &s.underline,
                            (
                                s.strike,
                                s.double_strike,
                                s.highlight,
                                &s.shading,
                                s.kind,
                                &s.vertical_align,
                            ),
                        )
                    })
                    .collect();
                let drawn = digest(&(
                    line.story.as_ref().map(|s| s.0),
                    line.baseline,
                    line.top,
                    line.pitch,
                    spans,
                ));
                (line.path.to_string(), line.story.clone(), drawn)
            })
            .collect();
        Some(PageDigests { rest, lines })
    }

    /// Page `page` of `version` as a reader sees it.
    pub fn page(&mut self, version: &LaidVersion, page: usize) -> Option<PageView> {
        let paint = self.paint(version, page)?;
        let blocks = version
            .facts()
            .pages
            .get(page)
            .map(|p| p.blocks.clone())
            .unwrap_or_default();
        Some(PageView {
            page,
            info: paint.geometry.info,
            frame: paint.geometry.frame,
            lines: paint.lines,
            rules: paint.rules,
            pictures: paint.pictures,
            drawings: paint.drawings,
            blocks,
            unpainted: paint.unpainted,
        })
    }

    /// Page `page` of `version` as SVG, byte for byte docx2svg's where nothing is listed
    /// beside it; the drawings filled by `drawings`. Beside it, what is not drawn as
    /// docx2svg draws it (empty where nothing): the version's reason, the page's own, and
    /// each drawing `drawings` left empty.
    pub fn page_svg(
        &mut self,
        version: &LaidVersion,
        page: usize,
        drawings: &dyn Drawings,
    ) -> Option<(String, Vec<String>)> {
        let paint = self.paint(version, page)?;
        let mut missing: Vec<String> = paint.unpainted.iter().map(|_| support::UNPAINTED.to_owned()).collect();
        let filled = Filled(
            paint
                .drawings
                .iter()
                .map(|slot| (slot.clone(), drawings.drawing(slot)))
                .collect(),
        );
        if filled.0.iter().any(|(_, drawn)| drawn.is_none()) {
            missing.push(support::DRAWINGS.to_owned());
        }
        let none = ooxml_word_layout::paint::svg::NoPictures;
        let cx = SvgContext {
            drawings: &filled,
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

/// A layout engine with no memos, measuring with `fonts`.
fn new_layout(fonts: &DocumentFonts, locale: &Locale, budget: &Arc<dyn CacheBudget>) -> WordLayout {
    let faces: Arc<dyn ooxml_word_layout::Faces> = Arc::clone(&fonts.glyphs) as Arc<dyn ooxml_word_layout::Faces>;
    WordLayout::new(faces, Arc::new(WordTables), locale.clone(), budget, KEY)
}

/// A page's drawings as their painters filled them, asked once each.
struct Filled(Vec<(DrawingSlot, Option<Vec<String>>)>);

impl Drawings for Filled {
    fn drawing(&self, slot: &DrawingSlot) -> Option<Vec<String>> {
        self.0
            .iter()
            .find(|(s, _)| s == slot)
            .and_then(|(_, drawn)| drawn.clone())
    }
}
