//! Owns: the painter of a page's drawings ([`WordDrawings`], the layout's `Drawings`
//! port): each drawing slot's graphic painted in its box as docx2svg's `_draw_graphic`
//! paints it -- pictures, shapes and groups ([`shapes`]), charts ([`chart`]), SmartArt
//! ([`diagram`]) -- its resources added to the page's, its markup written as `svg._float`
//! writes a floating drawing's group ([`write`](mod@write)); and what docx2svg says of it (`drawing-*`,
//! `chart-*`, `diagram-*` warnings, its messages, in the order met).
//!
//! A drawing's text boxes are found here ([`WordDrawings::text_frames`], [`text_boxes`]);
//! the layout lays their text out and the page writes it in the drawing's group.
//!
//! Must not: lay out or place a drawing (the layout has: [`DrawingSlot`]), hold state
//! between pages beyond the version's theme, or read anything but the version's package.

pub mod chart;
pub mod colours;
pub mod diagram;
pub mod shapes;
pub mod text_boxes;
pub mod text_effects;
pub mod vml;
pub mod write;

use crate::pictures::PackagePictures;
use colours::{colour_map_slots, Said, WordColours};
use ooxml_drawingml::source::FormatScheme;
use ooxml_render::Painter;
use ooxml_word_layout::geometry::text_box::TextBoxFrame;
use ooxml_word_layout::paint::drawings::{DrawingSlot, SlotPaths};
use ooxml_word_layout::paint::svg::{Drawings, PageDefs};
use ooxml_word_layout::paint::TextEffects;
use ooxml_word_model::model::Graphic;
use ooxml_word_model::theme::FontScheme;
use ooxml_word_model::value::Text;
use ooxml_word_model::WordDocument;
use shapes::{Draw, Drawn, Frames};
use std::collections::BTreeMap;
use std::sync::Arc;
use vml::Inks;
use write::{ImageSource, Primitive, PxBox};

const THEME: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
const NOTES: [(&str, &str); 2] = [
    (
        "footnote",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes",
    ),
    (
        "endnote",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes",
    ),
];

/// What one drawing painted: its group's lines, and what is said of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Painted {
    /// The lines of its group (`_float`'s, between the group's tags).
    pub lines: Vec<String>,
    /// docx2svg's layout warnings of it: `(code, message)`, in the order met.
    pub warnings: Vec<(String, String)>,
    /// What writing its pictures said (`picture-not-drawn`): `(code, message)`.
    pub pictures: Vec<(String, String)>,
    /// Its text path is drawn stretched by its advances, its outlines not at hand (said as
    /// an approximation, once per drawing).
    pub approximated: bool,
    /// The faces its text is drawn in (a text path's): `(face, bold, italic)`, for a
    /// rasteriser to be given.
    pub faces: Vec<(String, bool, bool)>,
}

/// One version's painter of drawings.
pub struct WordDrawings {
    pictures: PackagePictures,
    theme: BTreeMap<Text, Text>,
    slots: BTreeMap<String, String>,
    formats: Option<FormatScheme>,
    fonts: Option<Arc<FontScheme>>,
    locale: ooxml_numfmt::Locale,
    inks: Option<Arc<dyn Inks>>,
}

impl std::fmt::Debug for WordDrawings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WordDrawings").finish_non_exhaustive()
    }
}

impl WordDrawings {
    /// The painter of `document`'s drawings, its parts read from `pictures`' package, a
    /// chart's numbers written in `numbers`, a text path's outlines read through `inks`.
    pub fn new(
        document: &WordDocument,
        pictures: PackagePictures,
        numbers: ooxml_numfmt::Locale,
        inks: Option<Arc<dyn Inks>>,
    ) -> WordDrawings {
        let package = pictures.package();
        let formats = package
            .relationships(&document.main_part)
            .ok()
            .and_then(|r| {
                r.iter()
                    .find(|r| ooxml_word_model::parts::rel_type_is(&r.rel_type, THEME))
                    .and_then(|r| r.target_part.clone())
            })
            .and_then(|part| package.tree(&part).ok().flatten())
            .map(|tree| ooxml_drawingml::read::theme::theme(tree.root()).formats);
        WordDrawings {
            theme: document.context.colours.scheme.clone(),
            slots: colour_map_slots(document.context.settings.color_map.iter().map(|(k, v)| (&**k, &**v))),
            formats,
            fonts: document.context.font_scheme.clone(),
            locale: numbers,
            inks,
            pictures,
        }
    }

    fn colours(&self) -> WordColours<'_> {
        WordColours {
            theme: &self.theme,
            slots: self.slots.clone(),
            formats: self.formats.as_ref(),
        }
    }

    /// The part a drawing sits in (`Frames.owner`): its story's, a note's notes part, else
    /// the main document part.
    pub(crate) fn owner(&self, part: Option<&str>, path: &str) -> Option<String> {
        if let Some(part) = part {
            return Some(part.to_owned());
        }
        let main = self.pictures.main_part();
        let first = path.split('/').next().unwrap_or("");
        for (prefix, kind) in NOTES {
            if first.starts_with(&format!("w:{prefix}[")) {
                let rels = self.pictures.package().relationships(main).ok()?;
                return rels
                    .iter()
                    .find(|r| ooxml_word_model::parts::rel_type_is(&r.rel_type, kind))
                    .and_then(|r| r.target_part.clone());
            }
        }
        Some(main.to_owned())
    }

    /// What drawing `slot`'s graphic, its paths `paths`, makes, its resources added to `defs`.
    fn drawn(&self, slot: &DrawingSlot, paths: &SlotPaths, defs: &mut PageDefs) -> Drawn {
        let colours = self.colours();
        let measurer = ooxml_render::table_measurer(&ooxml_render::WORD);
        let mut painter = Painter::new(ooxml_render::WORD, &measurer);
        painter.dpi = shapes::DPI;
        painter.scope_ids(defs.scope());
        painter.resume_ids(defs.ids);
        let part = slot.story.as_ref().map(|(_, p)| p.to_string());
        let frames = PartFrames {
            drawings: self,
            colours: &colours,
            part: part.as_deref(),
        };
        let minor = self
            .fonts
            .as_deref()
            .map(|f| f.minor.latin.to_string())
            .filter(|m| !m.is_empty());
        let mut draw = Draw {
            colours: &colours,
            painter: &mut painter,
            frames: &frames,
            inks: self.inks.as_deref(),
            minor: minor.as_deref(),
            drawn: Drawn::default(),
        };
        let at = PxBox {
            x: slot.x,
            y: slot.y,
            width: slot.width,
            height: slot.height,
        };
        draw.graphic(slot.graphic.as_ref(), &at, &paths.path);
        let mut drawn = draw.drawn;
        for p in &mut drawn.primitives {
            if let Some(path) = p.path_mut().filter(|p| **p != paths.path) {
                *path = format!("{}/{path}", paths.paragraph);
            }
        }
        defs.ids = painter.ids();
        defs.defs
            .extend(painter.defs.iter().map(ooxml_scene::markup::Element::to_svg));
        drawn
    }

    /// The text boxes of `slot`'s drawing, its paths `paths`, where the layout lays their
    /// text out: each shape holding text in its box as the drawing is painted.
    pub fn text_frames(&self, slot: &DrawingSlot, paths: &SlotPaths) -> Vec<TextBoxFrame> {
        if !slot.graphic.as_deref().is_some_and(text_boxes::holds_text) {
            return Vec::new();
        }
        let drawn = self.drawn(slot, paths, &mut PageDefs::default());
        let colours = self.colours();
        drawn
            .text_boxes
            .iter()
            .map(|(graphic, at)| text_boxes::frame(graphic, at, &colours))
            .collect()
    }

    /// `slot`'s drawing painted, its paths `paths`, its resources added to `defs`.
    pub fn paint(&self, slot: &DrawingSlot, paths: &SlotPaths, defs: &mut PageDefs) -> Painted {
        let part = slot.story.as_ref().map(|(_, p)| p.to_string());
        let Drawn {
            primitives,
            said,
            faces,
            ..
        } = self.drawn(slot, paths, defs);
        let source = ImageSource {
            pictures: &self.pictures,
            part: part.as_deref(),
        };
        let (lines, pictures) = write::lines(&primitives, &source, defs);
        Painted {
            lines,
            warnings: messages(&said, &paths.path),
            pictures,
            approximated: said.iter().any(|c| c == TEXT_PATH_APPROXIMATE),
            faces,
        }
    }
}

/// What a text path drawn stretched by its advances says (an approximation, not a warning).
const TEXT_PATH_APPROXIMATE: &str = "drawing-approximate:text path";

impl WordDrawings {
    /// The faces `slot`'s text is drawn in (a text path's), its paths `paths`: `(face,
    /// bold, italic)`, for a rasteriser to be given (docx2svg's `drawn_faces`).
    pub fn faces(&self, slot: &DrawingSlot, paths: &SlotPaths) -> Vec<(String, bool, bool)> {
        fn written(g: &Graphic) -> bool {
            matches!(&*g.kind, "chart" | "diagram") || g.children.iter().any(|c| written(c))
        }
        if slot.graphic.as_deref().is_some_and(written) {
            // A chart's or a diagram's text: its faces are what its renderer wrote.
            return self.paint(slot, paths, &mut PageDefs::default()).faces;
        }
        let Some(graphic) = slot.graphic.as_deref().filter(|g| &*g.kind == "textpath") else {
            return Vec::new();
        };
        let minor = self
            .fonts
            .as_deref()
            .map(|f| f.minor.latin.to_string())
            .filter(|m| !m.is_empty());
        let text = graphic
            .source
            .as_ref()
            .and_then(|s| vml::TextPath::of(s, minor.as_deref()))
            .filter(|t| t.fit != Some(false));
        text.map(|t| vec![t.drawn_face(t.ink(self.inks.as_deref()).as_ref())])
            .unwrap_or_default()
    }
}

/// The warnings said of a drawing, once each, as docx2svg words them (`_draw_graphic`); a
/// text path's approximation is said apart ([`Painted::approximated`]).
fn messages(said: &Said, path: &str) -> Vec<(String, String)> {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for code in said {
        if seen.contains(&code.as_str()) || code == TEXT_PATH_APPROXIMATE {
            continue;
        }
        seen.push(code);
        let (kind, what) = code.split_once(':').unwrap_or((code, ""));
        let what = if what.is_empty() { "content" } else { what };
        out.push((
            kind.to_owned(),
            format!("a floating drawing's {what} is not drawn faithfully ({path})"),
        ));
    }
    out
}

impl Drawings for WordDrawings {
    fn drawing(&self, slot: &DrawingSlot, paths: &SlotPaths, defs: &mut PageDefs) -> Option<Vec<String>> {
        Some(self.paint(slot, paths, defs).lines)
    }

    fn text_paint(
        &self,
        effects: &TextEffects,
        bounds: (f64, f64, f64, f64),
        colour: Option<&str>,
        defs: &mut PageDefs,
    ) -> Option<String> {
        Some(text_effects::paint(&self.colours(), effects, (bounds, colour), defs))
    }
}

/// Charts and diagrams, related from the part the drawing sits in.
struct PartFrames<'a> {
    drawings: &'a WordDrawings,
    colours: &'a WordColours<'a>,
    part: Option<&'a str>,
}

impl Frames for PartFrames<'_> {
    fn draw(
        &self,
        graphic: &Graphic,
        at: &PxBox,
        path: &str,
        painter: &mut Painter<'_>,
        said: &mut Said,
    ) -> Option<Vec<Primitive>> {
        let Some(owner) = self.drawings.owner(self.part, path) else {
            said.push(format!("drawing-not-drawn:{}", graphic.kind));
            return None;
        };
        let package = self.drawings.pictures.package();
        if &*graphic.kind == "diagram" {
            let job = diagram::DiagramJob {
                colours: self.colours,
                fonts: self.drawings.fonts.as_deref(),
            };
            let relationship = graphic.relationship.as_deref();
            return diagram::draw(&job, (package, &owner, relationship), (at, path), painter, said);
        }
        if &*graphic.kind != "chart" {
            return None;
        }
        let target = graphic.relationship.as_deref().and_then(|r| {
            let rels = package.relationships(&owner).ok()?;
            rels.get(r)?.target_part.clone()
        });
        let Some(target) = target.filter(|t| package.has_part(t)) else {
            said.push("chart-unreadable:no chart part".into());
            return None;
        };
        let Ok(Some(tree)) = package.tree(&target) else {
            said.push("chart-unreadable:not well-formed".into());
            return None;
        };
        let job = chart::ChartJob {
            colours: self.colours,
            fonts: self.drawings.fonts.as_deref(),
            locale: &self.drawings.locale,
        };
        chart::draw(&job, tree.root(), (at, path), painter, said)
    }
}
