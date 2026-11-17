//! Owns: the painter of a page's drawings ([`WordDrawings`], the layout's `Drawings`
//! port): each drawing slot's graphic painted in its box as docx2svg's `_draw_graphic`
//! paints it -- pictures, shapes and groups ([`shapes`]), charts ([`chart`]) -- its
//! resources added to the page's, its markup written as `svg._float` writes a floating
//! drawing's group ([`write`]); and what docx2svg says of it (`drawing-*`, `chart-*`
//! warnings, its messages, in the order met).
//!
//! What the layout does not lay out yet in a drawing (a text box's text, SmartArt, VML
//! shapes and text paths) is found before a version is laid out ([`crate::support`]); such
//! a document never reaches this painter.
//!
//! Must not: lay out or place a drawing (the layout has: [`DrawingSlot`]), hold state
//! between pages beyond the version's theme, or read anything but the version's package.

pub mod chart;
pub mod colours;
pub mod shapes;
pub mod write;

use crate::pictures::PackagePictures;
use colours::{colour_map_slots, Said, WordColours};
use ooxml_drawingml::source::FormatScheme;
use ooxml_render::Painter;
use ooxml_word_layout::paint::drawings::DrawingSlot;
use ooxml_word_layout::paint::svg::{Drawings, PageDefs};
use ooxml_word_model::model::Graphic;
use ooxml_word_model::theme::FontScheme;
use ooxml_word_model::value::Text;
use ooxml_word_model::WordDocument;
use shapes::{Draw, Drawn, Frames};
use std::collections::BTreeMap;
use std::sync::Arc;
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
}

/// One version's painter of drawings.
pub struct WordDrawings {
    pictures: PackagePictures,
    theme: BTreeMap<Text, Text>,
    slots: BTreeMap<String, String>,
    formats: Option<FormatScheme>,
    fonts: Option<Arc<FontScheme>>,
    locale: ooxml_numfmt::Locale,
}

impl std::fmt::Debug for WordDrawings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WordDrawings").finish_non_exhaustive()
    }
}

impl WordDrawings {
    /// The painter of `document`'s drawings, its parts read from `pictures`' package.
    pub fn new(document: &WordDocument, pictures: PackagePictures) -> WordDrawings {
        let package = pictures.package();
        let formats = package
            .relationships(&document.main_part)
            .ok()
            .and_then(|r| {
                r.iter()
                    .find(|r| r.rel_type == THEME)
                    .and_then(|r| r.target_part.clone())
            })
            .and_then(|part| package.tree(&part).ok().flatten())
            .map(|tree| ooxml_drawingml::read::theme::theme(tree.root()).formats);
        WordDrawings {
            theme: document.context.colours.scheme.clone(),
            slots: colour_map_slots(document.context.settings.color_map.iter().map(|(k, v)| (&**k, &**v))),
            formats,
            fonts: document.context.font_scheme.clone(),
            locale: ooxml_numfmt::EN_US,
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
    fn owner(&self, part: Option<&str>, path: &str) -> Option<String> {
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
                    .find(|r| r.rel_type == kind)
                    .and_then(|r| r.target_part.clone());
            }
        }
        Some(main.to_owned())
    }

    /// `slot`'s drawing painted, its resources added to `defs`.
    pub fn paint(&self, slot: &DrawingSlot, defs: &mut PageDefs) -> Painted {
        let colours = self.colours();
        let measurer = ooxml_render::table_measurer(&ooxml_render::WORD);
        let mut painter = Painter::new(ooxml_render::WORD, &measurer);
        painter.dpi = shapes::DPI;
        painter.resume_ids(defs.ids);
        let part = slot.story.as_ref().map(|(_, p)| p.to_string());
        let frames = PartFrames {
            drawings: self,
            colours: &colours,
            part: part.as_deref(),
        };
        let mut draw = Draw {
            colours: &colours,
            painter: &mut painter,
            frames: &frames,
            drawn: Drawn::default(),
        };
        let at = PxBox {
            x: slot.x,
            y: slot.y,
            width: slot.width,
            height: slot.height,
        };
        draw.graphic(slot.graphic.as_ref(), &at, &slot.path);
        let Drawn {
            mut primitives, said, ..
        } = draw.drawn;
        for p in &mut primitives {
            if let Some(path) = p.path_mut().filter(|p| **p != slot.path) {
                *path = format!("{}/{path}", slot.paragraph_path);
            }
        }
        defs.ids = painter.ids();
        defs.defs
            .extend(painter.defs.iter().map(ooxml_scene::markup::Element::to_svg));
        let source = ImageSource {
            pictures: &self.pictures,
            part: part.as_deref(),
        };
        let (lines, pictures) = write::lines(&primitives, &source);
        Painted {
            lines,
            warnings: messages(&said, &slot.path),
            pictures,
        }
    }
}

/// The warnings said of a drawing, once each, as docx2svg words them (`_draw_graphic`).
fn messages(said: &Said, path: &str) -> Vec<(String, String)> {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for code in said {
        if seen.contains(&code.as_str()) {
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
    fn drawing(&self, slot: &DrawingSlot, defs: &mut PageDefs) -> Option<Vec<String>> {
        Some(self.paint(slot, defs).lines)
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
        if &*graphic.kind != "chart" {
            return None;
        }
        let package = self.drawings.pictures.package();
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
