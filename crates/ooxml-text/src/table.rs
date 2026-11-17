//! Owns: DrawingML's table measure -- a string's width from a face's measured advance table
//! and its kern pairs, with PowerPoint's rules for bold (the bold table, else the upright
//! widened by 5 %; an East Asian character synthetically emboldened from 16 pt) and for
//! characters a table lacks (its default or East Asian width; with no table at all a ratio
//! of the em by kind) -- ooxml-common's `text.measure.DefaultTextMeasurer.measure_text_width`
//! with the OpenType kern feature.
//!
//! Must not: find a face (it reads one through [`MeasuredFace`], which a caller adapts the
//! fonts layer's tables to), or break lines.

/// What the measure reads of a face's measured table.
pub trait MeasuredFace {
    /// Units per em.
    fn units_per_em(&self) -> u32;
    /// The advance of `ch` in the upright or bold table, where the table has one.
    fn advance(&self, ch: char, bold: bool) -> Option<u32>;
    /// The advance of a character with no entry, upright or bold.
    fn default_width(&self, bold: bool) -> u32;
    /// The advance of an East Asian character with no entry, upright or bold.
    fn cjk_width(&self, bold: bool) -> u32;
    /// Whether the table has bold advances.
    fn has_bold_widths(&self) -> bool;
    /// Whether the bold advances say nothing the upright ones do not.
    fn bold_is_indistinguishable(&self) -> bool;
    /// The kern adjustment between two characters, font units (0 for a face that does not
    /// kern); `bold` asks for the bold pairs where the face has them.
    fn kern(&self, left: char, right: char, bold: bool) -> i32;
}

/// Pixels per point.
const PX_PER_PT: f64 = 96.0 / 72.0;
/// How much wider a bold run is drawn when the face has no bold table.
const BOLD_FACTOR: f64 = 1.05;
/// The width PowerPoint's synthetic bold adds to an East Asian character, points ...
const EAST_ASIAN_SYNTHETIC_BOLD_PT: f64 = 0.125;
/// ... from this size, points.
const EAST_ASIAN_SYNTHETIC_BOLD_FROM_PT: f64 = 16.0;
const NARROW_RATIO: f64 = 0.3;
const NORMAL_RATIO: f64 = 0.6;
const WIDE_RATIO: f64 = 1.0;
const NARROW_CHARS: &str = " !,.:;ijl1|'()[]{}";

/// Whether a code point is in the East Asian ranges that are full width.
pub fn is_cjk(ch: char) -> bool {
    let c = u32::from(ch);
    (0x3000..=0x9FFF).contains(&c)
        || (0xF900..=0xFAFF).contains(&c)
        || (0xFF01..=0xFF60).contains(&c)
        || (0x20000..=0x2A6DF).contains(&c)
}

/// The width of `text` at `size_pt` points, in pixels: each character's advance from
/// `face`'s table (or the ratio rules with none), plus the kern pairs between them.
pub fn measure_width<F: MeasuredFace>(text: &str, size_pt: f64, bold: bool, face: Option<&F>) -> f64 {
    let base = size_pt * PX_PER_PT;
    let synthetic = if bold && size_pt >= EAST_ASIAN_SYNTHETIC_BOLD_FROM_PT {
        EAST_ASIAN_SYNTHETIC_BOLD_PT * PX_PER_PT
    } else {
        0.0
    };
    let mut total = 0.0;
    let mut previous: Option<char> = None;
    for ch in text.chars() {
        let east_asian = is_cjk(ch);
        if let (Some(f), Some(prev)) = (face, previous) {
            total += kern_px(f, prev, ch, base, bold);
        }
        previous = Some(ch);
        let width = match face {
            None => {
                let ratio = if east_asian {
                    WIDE_RATIO
                } else if NARROW_CHARS.contains(ch) {
                    NARROW_RATIO
                } else {
                    NORMAL_RATIO
                };
                let w = base * ratio;
                if bold && !east_asian {
                    w * BOLD_FACTOR
                } else {
                    w
                }
            }
            Some(f) if bold && f.has_bold_widths() => {
                let w = advance_px(f, ch, east_asian, base, true);
                if east_asian && f.bold_is_indistinguishable() {
                    w + synthetic
                } else {
                    w
                }
            }
            Some(f) => {
                let w = advance_px(f, ch, east_asian, base, false);
                if bold && !east_asian {
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

fn kern_px<F: MeasuredFace>(face: &F, left: char, right: char, base: f64, bold: bool) -> f64 {
    let units = face.kern(left, right, bold && face.has_bold_widths());
    if units == 0 {
        return 0.0;
    }
    f64::from(units) / f64::from(face.units_per_em()) * base
}

fn advance_px<F: MeasuredFace>(face: &F, ch: char, east_asian: bool, base: f64, bold: bool) -> f64 {
    let width = face.advance(ch, bold).unwrap_or_else(|| {
        if east_asian {
            face.cjk_width(bold)
        } else {
            face.default_width(bold)
        }
    });
    f64::from(width) / f64::from(face.units_per_em()) * base
}
