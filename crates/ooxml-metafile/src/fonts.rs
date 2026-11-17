//! Owns: what a metafile's text asks of fonts -- a run's font attributes (the `font-family`
//! stack under the font mapping, the generic family a `LOGFONT`'s pitch and family asks
//! for), a face's ascent, descent and line gap as fractions of the em, and a string's width
//! -- answered from the fonts layer's measured Office tables (`ooxml-fonts`' `office` and
//! `css`) and the text engine's table measure (`ooxml-text`'s `table`), adapted here.
//!
//! Must not: hold anything but the font mapping.

use ooxml_fonts::{font_family_value, office_metrics_for, FontMapping, OfficeFace};
use ooxml_scene::{Decoration, Font};
use ooxml_text::{measure_width, MeasuredFace};

/// Arial's ascent and descent (1854 and 434 of 2048), for a face with no measured table.
const ARIAL: (f64, f64) = (1854.0 / 2048.0, 434.0 / 2048.0);
/// Arial's line gap (67 of 2048).
const ARIAL_GAP: f64 = 67.0 / 2048.0;

/// A run's style.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FontStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

/// The font answers a metafile's text is drawn with.
#[derive(Debug, Clone)]
pub(crate) struct Faces {
    mapping: FontMapping,
}

impl Faces {
    pub fn new(mapping: FontMapping) -> Faces {
        Faces { mapping }
    }

    /// A text run's font: `face` (Arial when empty) at `size`, the generic family the
    /// pitch and family byte names replacing the stack's last when it differs.
    pub fn font(&self, face: &str, size: f64, style: FontStyle, pitch_family: u8) -> Font {
        let name = if face.is_empty() { "Arial" } else { face };
        let mut family = font_family_value(&[name], &self.mapping);
        let generic = generic(pitch_family);
        if let Some(f) = family.as_mut() {
            if !f.ends_with(generic) && pitch_family & 0xF0 != 0 {
                let head = f.rsplit_once(", ").map_or(f.as_str(), |(h, _)| h).to_owned();
                *f = format!("{head}, {generic}");
            }
        }
        let mut decorations = Vec::new();
        if style.underline {
            decorations.push(Decoration::Underline);
        }
        if style.strike {
            decorations.push(Decoration::LineThrough);
        }
        Font {
            size,
            family,
            bold: style.bold,
            italic: style.italic,
            decorations,
        }
    }

    fn table(face: &str) -> Option<&'static OfficeFace> {
        if face.is_empty() {
            return None;
        }
        office_metrics_for(face).filter(|t| t.units_per_em != 0)
    }

    /// Ascent and descent as fractions of the em: the face's measured table, else Arial's.
    pub fn vertical_metrics(&self, face: &str) -> (f64, f64) {
        Self::table(face).map_or(ARIAL, |t| {
            let upm = f64::from(t.units_per_em);
            (f64::from(t.ascender) / upm, f64::from(t.descender).abs() / upm)
        })
    }

    /// Ascent, descent and line gap as fractions of the em.
    pub fn line_metrics(&self, face: &str) -> (f64, f64, f64) {
        let (ascent, descent) = self.vertical_metrics(face);
        let gap = Self::table(face)
            .and_then(|t| t.line_gap.map(|g| f64::from(g) / f64::from(t.units_per_em)))
            .unwrap_or(ARIAL_GAP);
        (ascent, descent, gap)
    }

    /// The width of `text` at `size_pt` points in `family`, pixels.
    pub fn width(&self, text: &str, size_pt: f64, bold: bool, family: &str) -> f64 {
        let face = office_metrics_for(family).map(Table);
        measure_width(text, size_pt, bold, face.as_ref())
    }
}

/// The generic family a `LOGFONT`'s pitch and family byte names.
fn generic(pitch_family: u8) -> &'static str {
    match pitch_family & 0xF0 {
        0x10 => "serif",
        0x30 => "monospace",
        0x40 => "cursive",
        0x50 => "fantasy",
        _ => "sans-serif",
    }
}

/// A measured Office table as the text engine's port.
struct Table(&'static OfficeFace);

impl MeasuredFace for Table {
    fn units_per_em(&self) -> u32 {
        self.0.units_per_em
    }

    fn advance(&self, ch: char, bold: bool) -> Option<u32> {
        self.0.advance(ch, bold)
    }

    fn default_width(&self, bold: bool) -> u32 {
        if bold {
            self.0.bold_default_width
        } else {
            self.0.default_width
        }
    }

    fn cjk_width(&self, bold: bool) -> u32 {
        if bold {
            self.0.bold_cjk_width
        } else {
            self.0.cjk_width
        }
    }

    fn has_bold_widths(&self) -> bool {
        self.0.has_bold_widths()
    }

    fn bold_is_indistinguishable(&self) -> bool {
        self.0.bold_is_indistinguishable()
    }

    fn kern(&self, left: char, right: char, bold: bool) -> i32 {
        self.0.kern(left, right, bold)
    }
}
