//! Owns: the Word layout's SVG differential -- each document laid out and painted by the
//! Rust layout over the fonts docx2svg found, every page written as SVG and compared byte
//! for byte with the page docx2svg wrote (`layout_differential.py --svg`); a page that
//! differs is named by its cause: what the paint does not draw yet (its first reason), or
//! the first element that differs. The pictures come from the package
//! ([`PackagePictures`]: a metafile drawn as SVG by `ooxml-metafile`, as docx2svg draws
//! one with ooxml-common).
//!
//! Must not: change what is compared to make it pass.

use crate::export::{Unlimited, KEY};
use crate::layout::LayoutRecords;
use crate::text::fonts_of;
use ooxml_base::{Limits, Locale};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_word_layout::paint::svg::{page_svg, Pictures, SvgContext};
use ooxml_word_layout::paint::{paint_page, FaceNames, GlyphSize, PaintContext};
use ooxml_word_layout::tables::WordTables;
use ooxml_word_layout::{WordGlyphs, WordLayout};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The main document part's pictures, read from the package.
pub struct PackagePictures {
    package: Package,
    main: String,
}

const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

impl PackagePictures {
    /// The pictures of the package in `bytes`.
    pub fn new(bytes: &[u8]) -> Option<PackagePictures> {
        let package = Package::open(Arc::from(bytes.to_vec()), &Limits::default()).ok()?;
        let main = package
            .relationships("")
            .ok()?
            .iter()
            .find(|r| r.rel_type == OFFICE)
            .and_then(|r| r.target_part.clone())?;
        Some(PackagePictures { package, main })
    }
}

impl Pictures for PackagePictures {
    fn picture(&self, relationship: &str, part: Option<&str>) -> Option<(String, Arc<[u8]>)> {
        let source = part.unwrap_or(&self.main);
        let rels = self.package.relationships(source).ok()?;
        let target = rels.get(relationship)?.target_part.clone()?;
        let bytes = self.package.read(&target).ok()??;
        let mime = self.package.content_types().ok()?.of(&target)?.to_owned();
        if matches!(mime.as_str(), "image/x-emf" | "image/emf" | "image/x-wmf" | "image/wmf") {
            if let Some(drawing) = ooxml_metafile::draw(&bytes, &ooxml_metafile::Options::default()) {
                if drawing.drawn > 0 {
                    return Some(("image/svg+xml".to_owned(), Arc::from(drawing.svg())));
                }
            }
        }
        Some((mime, bytes))
    }
}

/// Faces named as docx2svg names them on this machine.
pub struct DiskNames(pub ooxml_fonts::DrawingFonts);

impl FaceNames for DiskNames {
    fn drawing_name(&self, family: &str, bold: bool, italic: bool) -> Option<ooxml_fonts::DrawingName> {
        self.0.drawing_name(family, bold, italic)
    }
}

/// What the SVG differential found.
#[derive(Debug, Default)]
pub struct SvgOutcome {
    /// Documents compared.
    pub documents: usize,
    /// Pages compared.
    pub pages: usize,
    /// Pages byte for byte equal.
    pub same: usize,
    /// Pages that differ, by cause, with an example (document, page, detail).
    pub causes: BTreeMap<String, (usize, String)>,
    /// The Rust layout's and paint's own time.
    pub time: Duration,
    /// The drawing index of the fonts folders, made once (every record names the same).
    pub index: Option<Arc<ooxml_fonts::DrawingIndex>>,
}

impl SvgOutcome {
    fn differ(&mut self, cause: String, example: String) {
        let entry = self.causes.entry(cause).or_insert((0, example));
        entry.0 += 1;
    }
}

/// The first line of two texts that differ, as `(line, ours, theirs)`.
fn first_line_difference(ours: &str, theirs: &str) -> String {
    for (k, (a, b)) in ours.lines().zip(theirs.lines()).enumerate() {
        if a != b {
            let cut = |s: &str| s.chars().take(240).collect::<String>();
            return format!("line {k}: {} | {}", cut(a), cut(b));
        }
    }
    format!("{} lines vs {}", ours.lines().count(), theirs.lines().count())
}

/// What kind of element a line of SVG is, for a cause.
fn element_kind(line: &str) -> String {
    let tag: String = line
        .trim_start_matches('<')
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    match line.find("data-docx-kind=\"") {
        Some(at) => {
            let rest = line.get(at + 16..).unwrap_or_default();
            format!("{tag} {}", rest.split('"').next().unwrap_or_default())
        }
        None => tag,
    }
}

/// Lay one document out, paint every page and compare it with docx2svg's.
pub fn run_svg_document(records: &LayoutRecords, outcome: &mut SvgOutcome) -> Result<(), String> {
    let (Some(theirs), Some(fonts)) = (&records.svg, &records.fonts) else {
        return Ok(());
    };
    outcome.documents += 1;
    let bytes = std::fs::read(&records.path).map_err(|e| format!("{}: {e}", records.path))?;
    let glyphs = Arc::new(WordGlyphs::new(fonts_of(fonts, Some(&bytes))?));
    let pictures = PackagePictures::new(&bytes);
    if outcome.index.is_none() {
        outcome.index = Some(Arc::new(crate::text::drawing_index_of(fonts)?));
    }
    let index = outcome.index.clone().ok_or("no drawing index")?;
    let names = DiskNames(crate::text::drawing_fonts_of(fonts, Some(&bytes), &index)?);
    let (mut model, document) = crate::export::open(bytes)?;
    let budget: Arc<dyn CacheBudget> = Arc::new(Unlimited);
    let mut engine = WordLayout::new(
        Arc::clone(&glyphs) as Arc<dyn ooxml_word_layout::Faces>,
        Arc::new(WordTables),
        Locale::default(),
        &budget,
        KEY,
    );
    let started = Instant::now();
    let laid = engine.lay_out(&mut model, &document);
    let cx = PaintContext {
        faces: &*glyphs,
        names: &names,
        glyph_size: GlyphSize::Device,
        decimal: '.',
    };
    let none = ooxml_word_layout::paint::svg::NoPictures;
    let svg_cx = SvgContext {
        names: &names,
        pictures: pictures.as_ref().map_or(&none as &dyn Pictures, |p| p as &dyn Pictures),
        glyph_size: GlyphSize::Device,
    };
    let mut ours = Vec::new();
    for page in 0..laid.page_count() {
        let stories = engine.page_stories(&mut model, &laid, page);
        ours.push(paint_page(&laid, &mut model, page, &stories, &cx).map(|p| (page_svg(&p, &svg_cx), p.unpainted)));
    }
    outcome.time += started.elapsed();
    if ours.len() != theirs.len() {
        outcome.differ(
            "page count".into(),
            format!("{}: {} vs {}", records.path, ours.len(), theirs.len()),
        );
    }
    for (k, (mine, theirs)) in ours.iter().zip(theirs).enumerate() {
        outcome.pages += 1;
        let Some((svg, unpainted)) = mine else {
            outcome.differ("not painted".into(), format!("{} page {k}", records.path));
            continue;
        };
        if svg == theirs {
            outcome.same += 1;
            continue;
        }
        let detail = first_line_difference(svg, theirs);
        let cause = match unpainted.first() {
            Some(reason) => format!("unpainted: {reason}"),
            None => {
                let line = theirs
                    .lines()
                    .zip(svg.lines())
                    .find(|(a, b)| a != b)
                    .map_or("", |(a, _)| a);
                format!("differs at {}", element_kind(line))
            }
        };
        outcome.differ(cause, format!("{} page {k}: {detail}", records.path));
    }
    Ok(())
}
