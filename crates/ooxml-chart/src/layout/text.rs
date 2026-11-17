//! Owns: chart text as the layout measures it -- a face's vertical metrics at a size
//! ([`FontBox`]), a face with its East Asian partner and colour ([`ChartFont`]), a line's
//! advance (a sum of upright advances with no kerning, GDI's `GetTextExtent`, measured), and
//! the breaks the chart engine makes: category labels at white space, legend entries at
//! spaces and between East Asian characters, and a turned label cut with an ellipsis.
//!
//! Must not: shape or kern (the renderer draws the text), or lay out a paragraph.

use super::consts::{
    CATEGORY_LABEL_GAP_EM, DEFAULT_LINE_HEIGHT_RATIO, FALLBACK_ASCENT, FALLBACK_DESCENT, LABEL_ELLIPSIS,
    LABEL_INK_CENTRE_EM, WRAPPED_LABEL_MAX_LINES,
};
use ooxml_drawingml::drawable::Colour;
use ooxml_fonts::office::{office_metrics_for, OfficeFace};
use ooxml_text::table::is_cjk;

/// The measured table a family is measured with (`fontmap.metrics_for`).
fn metrics_for(family: Option<&str>) -> Option<&'static OfficeFace> {
    family.and_then(office_metrics_for)
}

/// Whether `c` is white space to Python's `str.split()` and `str.strip()`.
pub fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.split()`.
pub fn py_split(text: &str) -> Vec<&str> {
    text.split(py_space).filter(|t| !t.is_empty()).collect()
}

/// Python's `str.strip()`.
pub fn py_strip(text: &str) -> &str {
    text.trim_matches(py_space)
}

/// The vertical metrics of one face at one size, points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontBox {
    /// The size.
    pub size: f64,
    /// The ascent.
    pub ascent: f64,
    /// The descent (positive).
    pub descent: f64,
    /// The `hhea` line gap (0 where unmeasured).
    pub gap: f64,
}

impl FontBox {
    /// One line's own box, with no leading.
    pub fn line_height(&self) -> f64 {
        self.ascent + self.descent
    }

    /// Baseline to baseline.
    pub fn pitch(&self) -> f64 {
        self.ascent + self.descent + self.gap
    }

    /// How far above its baseline a label's optical centre sits (fitted).
    pub fn ink_centre(&self) -> f64 {
        LABEL_INK_CENTRE_EM * self.size
    }

    /// Where the text engine puts the first baseline below a box's top.
    pub fn first_baseline(&self) -> f64 {
        (DEFAULT_LINE_HEIGHT_RATIO - self.descent / self.size) * self.size
    }
}

/// A face's box at `size`: its table's metrics, or the fallback ratios.
pub fn font_box(family: Option<&str>, size: f64) -> FontBox {
    let Some(metrics) = metrics_for(family) else {
        return FontBox {
            size,
            ascent: FALLBACK_ASCENT * size,
            descent: FALLBACK_DESCENT * size,
            gap: 0.0,
        };
    };
    let units = f64::from(metrics.units_per_em);
    FontBox {
        size,
        ascent: f64::from(metrics.ascender) / units * size,
        descent: f64::from(metrics.descender).abs() / units * size,
        gap: f64::from(metrics.line_gap.unwrap_or(0)) / units * size,
    }
}

/// The face a piece of chart text is drawn in, with its metrics and colour.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartFont {
    /// The Latin face.
    pub family: Option<String>,
    /// Its box at the size.
    pub box_: FontBox,
    /// The face East Asian characters resolve to.
    pub family_ea: Option<String>,
    /// That face's box.
    pub box_ea: Option<FontBox>,
    /// The colour its text cascade states, if any.
    pub colour: Option<Colour>,
}

impl ChartFont {
    /// The size, points.
    pub fn size(&self) -> f64 {
        self.box_.size
    }

    /// `text`'s advance, points.
    pub fn width(&self, text: &str) -> f64 {
        text_width(text, self.family.as_deref(), self.box_.size, self.family_ea.as_deref())
    }

    /// The line box of the face that draws `texts`: the East Asian one where any is.
    pub fn box_for<'a>(&self, texts: impl IntoIterator<Item = &'a str>) -> FontBox {
        if let Some(ea) = self.box_ea {
            if texts.into_iter().any(|t| t.chars().any(is_cjk)) {
                return ea;
            }
        }
        self.box_
    }
}

/// One line's advance, points: each character's upright advance from its face's table (the
/// East Asian face's for an East Asian character), half an em without a table.
pub fn text_width(text: &str, family: Option<&str>, size: f64, family_ea: Option<&str>) -> f64 {
    let metrics = metrics_for(family);
    let ea_metrics = family_ea.filter(|f| !f.is_empty()).and_then(|f| metrics_for(Some(f)));
    if metrics.is_none() && ea_metrics.is_none() {
        #[allow(clippy::cast_precision_loss)]
        let count = text.chars().count() as f64;
        return 0.5 * size * count;
    }
    let mut total = 0.0;
    for c in text.chars() {
        let east_asian = is_cjk(c);
        let table = if east_asian && ea_metrics.is_some() {
            ea_metrics
        } else {
            metrics
        };
        let Some(table) = table else {
            total += 0.5 * size;
            continue;
        };
        let width = table.advance(c, false).unwrap_or(if east_asian {
            table.cjk_width
        } else {
            table.default_width
        });
        total += f64::from(width) / f64::from(table.units_per_em) * size;
    }
    total
}

/// One category label broken to its band at white space, greedily, at most six lines.
pub fn wrap_label(text: &str, font: &ChartFont, band: f64) -> Vec<String> {
    let tokens = py_split(text);
    let Some((first, rest)) = tokens.split_first() else {
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![text.to_owned()]
        };
    };
    if band <= 0.0 {
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![text.to_owned()]
        };
    }
    let mut lines = Vec::new();
    let mut current = (*first).to_owned();
    for token in rest {
        let candidate = format!("{current} {token}");
        if font.width(&candidate) <= band {
            current = candidate;
        } else {
            lines.push(std::mem::replace(&mut current, (*token).to_owned()));
        }
    }
    lines.push(current);
    lines.truncate(WRAPPED_LABEL_MAX_LINES);
    lines
}

/// `text` split where a legend entry may break: a space, and either side of an East Asian
/// character.
fn legend_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    for c in text.chars() {
        if c == ' ' || is_cjk(c) {
            if !word.is_empty() {
                tokens.push(std::mem::take(&mut word));
            }
            tokens.push(c.to_string());
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        tokens.push(word);
    }
    tokens
}

/// One legend entry broken greedily to `column`.
pub fn legend_wrap(text: &str, font: &ChartFont, column: f64) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    if column <= 0.0 || font.width(text) <= column {
        return vec![text.to_owned()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for token in legend_tokens(text) {
        if token == " " {
            if !current.is_empty() {
                current.push(' ');
            }
            continue;
        }
        let candidate = format!("{current}{token}");
        if !py_strip(&current).is_empty() && font.width(&candidate) > column {
            lines.push(current.trim_end_matches(py_space).to_owned());
            current = token;
        } else {
            current = candidate;
        }
    }
    if !py_strip(&current).is_empty() {
        lines.push(current.trim_end_matches(py_space).to_owned());
    }
    if lines.is_empty() {
        vec![text.to_owned()]
    } else {
        lines
    }
}

/// How far below the axis line a turned label's far end sits.
pub fn rotated_label_anchor(box_: &FontBox) -> f64 {
    box_.ascent * super::consts::sin_45() + CATEGORY_LABEL_GAP_EM * box_.size
}

/// A turned category label cut to the longest prefix that fits with an ellipsis (at least
/// one character).
pub fn truncate_label(text: &str, font: &ChartFont, allowance: f64) -> String {
    if text.is_empty() || font.width(text) <= allowance {
        return text.to_owned();
    }
    let ellipsis = font.width(LABEL_ELLIPSIS);
    let chars: Vec<char> = text.chars().collect();
    let mut kept = 0;
    for index in 1..chars.len() {
        let prefix: String = chars.iter().take(index).collect();
        if font.width(&prefix) + ellipsis > allowance {
            break;
        }
        kept = index;
    }
    let prefix: String = chars.iter().take(kept.max(1)).collect();
    format!("{prefix}{LABEL_ELLIPSIS}")
}

/// A line broken at single spaces to `cap` (`_wrap_to_width`).
pub fn wrap_to_width(text: &str, font: &ChartFont, cap: f64) -> Vec<String> {
    if text.is_empty() || cap <= 0.0 || font.width(text) <= cap {
        return vec![text.to_owned()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if !current.is_empty() && font.width(&candidate) > cap {
            lines.push(std::mem::replace(&mut current, word.to_owned()));
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        vec![text.to_owned()]
    } else {
        lines
    }
}
