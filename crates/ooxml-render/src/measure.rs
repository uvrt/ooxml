//! Owns: the text measure the painter lays text out with -- a port ([`TextMeasurer`]) and
//! its default ([`TableMeasurer`]: ooxml-common's `text.measure.DefaultTextMeasurer`), which
//! measures from the Office tables `ooxml-fonts` holds, East Asian characters from the
//! run's East Asian face, kerned from the table the application's [`KerningSource`] names.
//!
//! Must not: read a font file, or break lines (the text layout does).

use ooxml_fonts::{office_metrics_for, KerningSource, OfficeFace};

/// Pixels per point.
pub const PX_PER_PT: f64 = 96.0 / 72.0;
/// A face's line box, in ems, whatever the face (`DEFAULT_LINE_HEIGHT_RATIO`).
pub const DEFAULT_LINE_HEIGHT_RATIO: f64 = 1.2;
/// The first baseline's depth, in ems, with no measured face.
pub const DEFAULT_ASCENDER_RATIO: f64 = 1.0;
const BOLD_FACTOR: f64 = 1.05;
const EAST_ASIAN_SYNTHETIC_BOLD_PT: f64 = 0.125;
const EAST_ASIAN_SYNTHETIC_BOLD_FROM_PT: f64 = 16.0;
const NARROW_CHARS: &str = " !,.:;ijl1|'()[]{}";

/// Whether a character is in the East Asian ranges that are full width and break between
/// any two characters.
pub fn is_cjk(ch: char) -> bool {
    let c = u32::from(ch);
    (0x3000..=0x9FFF).contains(&c)
        || (0xF900..=0xFAFF).contains(&c)
        || (0xFF01..=0xFF60).contains(&c)
        || (0x20000..=0x2A6DF).contains(&c)
}

/// A run's faces as the measure asks for them.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Faces<'a> {
    /// The Latin face.
    pub latin: Option<&'a str>,
    /// The East Asian face.
    pub east_asian: Option<&'a str>,
}

/// What the painter asks of a text measure.
pub trait TextMeasurer: Send + Sync {
    /// The width of `text` at `size_pt`, px.
    fn width(&self, text: &str, size_pt: f64, bold: bool, faces: Faces<'_>) -> f64;
    /// What kerning takes off the join of `left`'s last character and `right`'s first, px.
    fn kern_between(&self, left: &str, right: &str, size_pt: f64, bold: bool, faces: Faces<'_>) -> f64;
    /// The line box, in ems.
    fn line_height_ratio(&self, faces: Faces<'_>) -> f64;
    /// The first baseline's depth below the line box's top, in ems.
    fn ascender_ratio(&self, faces: Faces<'_>) -> f64;
}

/// The measure from the Office tables, kerning from the table `kerning` names.
#[derive(Debug, Clone, Copy)]
pub struct TableMeasurer {
    /// Which kern pairs are charged.
    pub kerning: KerningSource,
}

fn metrics(family: Option<&str>) -> Option<&'static OfficeFace> {
    family.and_then(office_metrics_for)
}

fn same(a: Option<&OfficeFace>, b: Option<&OfficeFace>) -> bool {
    matches!((a, b), (Some(x), Some(y)) if std::ptr::eq(x, y))
}

impl TableMeasurer {
    fn kern_px(&self, first: char, second: char, base: f64, face: &OfficeFace, bold: bool) -> f64 {
        if !face.kerns_from(self.kerning) {
            return 0.0;
        }
        let units = face.kern_from(self.kerning, first, second, bold && face.has_bold_widths());
        if units == 0 {
            return 0.0;
        }
        f64::from(units) / f64::from(face.units_per_em) * base
    }
}

fn advance(ch: char, base: f64, face: &OfficeFace, bold: bool) -> f64 {
    let width = face.advance(ch, bold).unwrap_or(match (is_cjk(ch), bold) {
        (true, true) => face.bold_cjk_width,
        (true, false) => face.cjk_width,
        (false, true) => face.bold_default_width,
        (false, false) => face.default_width,
    });
    f64::from(width) / f64::from(face.units_per_em) * base
}

fn heuristic(ch: char) -> f64 {
    if is_cjk(ch) {
        1.0
    } else if NARROW_CHARS.contains(ch) {
        0.3
    } else {
        0.6
    }
}

impl TextMeasurer for TableMeasurer {
    fn width(&self, text: &str, size_pt: f64, bold: bool, faces: Faces<'_>) -> f64 {
        let base = size_pt * PX_PER_PT;
        let latin = metrics(faces.latin);
        let east_asian = metrics(faces.east_asian);
        let synthetic = if bold && size_pt >= EAST_ASIAN_SYNTHETIC_BOLD_FROM_PT {
            EAST_ASIAN_SYNTHETIC_BOLD_PT * PX_PER_PT
        } else {
            0.0
        };
        let mut total = 0.0;
        let mut previous: Option<(char, Option<&OfficeFace>)> = None;
        for ch in text.chars() {
            let cjk = is_cjk(ch);
            let face = if cjk && east_asian.is_some() { east_asian } else { latin };
            if let (Some(f), Some((prev, prev_face))) = (face, previous) {
                if same(Some(f), prev_face) {
                    total += self.kern_px(prev, ch, base, f, bold);
                }
            }
            previous = Some((ch, face));
            let width = match face {
                None => {
                    let w = base * heuristic(ch);
                    if bold && !cjk {
                        w * BOLD_FACTOR
                    } else {
                        w
                    }
                }
                Some(f) if bold && f.has_bold_widths() => {
                    let w = advance(ch, base, f, true);
                    if cjk && f.bold_is_indistinguishable() {
                        w + synthetic
                    } else {
                        w
                    }
                }
                Some(f) => {
                    let w = advance(ch, base, f, false);
                    if bold && !cjk {
                        w * BOLD_FACTOR
                    } else if bold {
                        w + synthetic
                    } else {
                        w
                    }
                }
            };
            total += width;
        }
        total
    }

    fn kern_between(&self, left: &str, right: &str, size_pt: f64, bold: bool, faces: Faces<'_>) -> f64 {
        let (Some(first), Some(second)) = (left.chars().last(), right.chars().next()) else {
            return 0.0;
        };
        let latin = metrics(faces.latin);
        let east_asian = metrics(faces.east_asian);
        let table = |ch: char| {
            if is_cjk(ch) && east_asian.is_some() {
                east_asian
            } else {
                latin
            }
        };
        let face = table(first);
        match face {
            Some(f) if same(face, table(second)) => self.kern_px(first, second, size_pt * PX_PER_PT, f, bold),
            _ => 0.0,
        }
    }

    fn line_height_ratio(&self, _faces: Faces<'_>) -> f64 {
        DEFAULT_LINE_HEIGHT_RATIO
    }

    fn ascender_ratio(&self, faces: Faces<'_>) -> f64 {
        match metrics(faces.latin).or_else(|| metrics(faces.east_asian)) {
            None => DEFAULT_ASCENDER_RATIO,
            Some(f) => {
                let descender = f64::from(f.descender.unsigned_abs()) / f64::from(f.units_per_em);
                (DEFAULT_LINE_HEIGHT_RATIO - descender).max(0.0)
            }
        }
    }
}
