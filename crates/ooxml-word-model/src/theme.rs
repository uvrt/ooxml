//! Owns: what the model reads of the theme part -- its font scheme ([`FontScheme`]) and colour
//! scheme -- and how WordprocessingML names a theme colour ([`ThemeColours`]): `w:themeColor`
//! / `w:themeFill` through `w:clrSchemeMapping`, with Word's own tint and shade arithmetic
//! ([`word_theme_tint`], measured in Word 16.106: Windows' integer HLS on a scale of 1530,
//! ooxml-common's `themetint`). Word draws the theme colour, not the `w:val` beside it.
//!
//! Must not: implement DrawingML colour transforms (`lumMod`, `lumOff`: phase P3's colour
//! engine); these attributes are not DrawingML transforms.

use crate::value::{text, Text};
use crate::w;
use crate::xmlread::{attr, elements, local};
use ooxml_xml::Element;
use std::collections::BTreeMap;

/// `a:majorFont` or `a:minorFont`: three typefaces and the per-script list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeFonts {
    /// `a:latin`'s typeface (`""` unstated).
    pub latin: Text,
    /// `a:ea`'s.
    pub east_asian: Text,
    /// `a:cs`'s.
    pub complex_script: Text,
    /// `a:font script=... typeface=...`, by script.
    pub scripts: BTreeMap<Text, Text>,
}

/// The theme's font scheme.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontScheme {
    /// The major (heading) fonts.
    pub major: ThemeFonts,
    /// The minor (body) fonts.
    pub minor: ThemeFonts,
}

impl FontScheme {
    /// The scheme Word uses for a package with no theme part: Calibri Light and Calibri,
    /// Latin only (measured, docx2svg's `BUILTIN_FONT_SCHEME`).
    pub fn builtin() -> FontScheme {
        let latin = |face: &str| ThemeFonts {
            latin: text(face),
            ..ThemeFonts::default()
        };
        FontScheme {
            major: latin("Calibri Light"),
            minor: latin("Calibri"),
        }
    }
}

fn theme_fonts(collection: Option<w::FontCollection<'_>>) -> ThemeFonts {
    let Some(c) = collection else {
        return ThemeFonts::default();
    };
    let face = |f: Option<w::TextFont<'_>>| text(f.and_then(w::TextFont::typeface).unwrap_or(""));
    let mut scripts = BTreeMap::new();
    for font in c.scripts() {
        if let Some(script) = font.script().filter(|s| !s.is_empty()) {
            scripts.insert(text(script), text(font.typeface().unwrap_or("")));
        }
    }
    ThemeFonts {
        latin: face(c.latin()),
        east_asian: face(c.east_asian()),
        complex_script: face(c.complex_script()),
        scripts,
    }
}

/// The theme part's font scheme, `None` where it has none.
pub fn font_scheme(root: &Element) -> Option<FontScheme> {
    let scheme = w::Theme::new(root).elements()?.fonts()?;
    Some(FontScheme {
        major: theme_fonts(scheme.major()),
        minor: theme_fonts(scheme.minor()),
    })
}

/// The theme's colour scheme: each slot (`dk1`, `lt1`, `accent1`...) as upper-case
/// `RRGGBB` -- an `a:srgbClr`'s value or an `a:sysClr`'s `lastClr`.
pub fn colour_scheme(root: &Element) -> BTreeMap<Text, Text> {
    let mut out = BTreeMap::new();
    let Some(scheme) = w::Theme::new(root).elements().and_then(w::ThemeElements::colours) else {
        return out;
    };
    for slot in elements(scheme.element()) {
        for colour in elements(slot) {
            let value = if local(colour) == "srgbClr" {
                attr(colour, "val")
            } else {
                attr(colour, "lastClr")
            };
            if let Some(v) = value.filter(|v| !v.is_empty()) {
                out.insert(text(local(slot)), text(&v.to_uppercase()));
            }
        }
    }
    out
}

/// How WordprocessingML theme colours read in one package: the theme's colours and the
/// settings' `w:clrSchemeMapping`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeColours {
    /// The colour scheme by slot (`dk1`...).
    pub scheme: BTreeMap<Text, Text>,
    /// `w:clrSchemeMapping`: `bg1` -> `light1`... as stated.
    pub mapping: BTreeMap<Text, Text>,
    /// Whether theme colours are read at all (a part read outside its package reads the
    /// stated values).
    pub active: bool,
}

fn slot_of(name: &str) -> &str {
    match name {
        "dark1" => "dk1",
        "light1" => "lt1",
        "dark2" => "dk2",
        "light2" => "lt2",
        "hyperlink" => "hlink",
        "followedHyperlink" => "folHlink",
        "accent1" | "accent2" | "accent3" | "accent4" | "accent5" | "accent6" => name,
        _ => "",
    }
}

impl ThemeColours {
    /// `RRGGBB` for a theme colour name with its tint or shade; `None` for a name the theme
    /// does not define (or when theme colours are not read).
    pub fn resolve(&self, name: &str, tint: Option<&str>, shade: Option<&str>) -> Option<Text> {
        if !self.active || name.is_empty() {
            return None;
        }
        let mapped = match name {
            "text1" => Some(("t1", "dark1")),
            "background1" => Some(("bg1", "light1")),
            "text2" => Some(("t2", "dark2")),
            "background2" => Some(("bg2", "light2")),
            _ => None,
        };
        let name = match mapped {
            Some((key, default)) => self
                .mapping
                .get(key)
                .map(|v| &**v)
                .filter(|v| !v.is_empty())
                .unwrap_or(default),
            None => name,
        };
        let base = self.scheme.get(slot_of(name))?;
        word_theme_tint(base, tint, shade).map(|c| text(&c))
    }
}

/// The settings' `w:clrSchemeMapping` attributes by local name.
pub fn colour_mapping(settings: &Element) -> BTreeMap<Text, Text> {
    let mut out = BTreeMap::new();
    if let Some(mapping) = w::Settings::new(settings).colour_mapping() {
        for a in mapping.element().attributes() {
            out.insert(text(a.name.local()), a.value.clone());
        }
    }
    out
}

const SCALE: i64 = 1530;
const SECTOR: i64 = SCALE / 6;

fn hex_byte(value: Option<&str>) -> Option<i64> {
    let v = value?;
    let n = i64::from_str_radix(v.trim(), 16).ok()?;
    (0..=255).contains(&n).then_some(n)
}

fn floor_div(a: i64, b: i64) -> i64 {
    a.div_euclid(b)
}

/// `base` (`RRGGBB`, a `#` allowed) with Word's `w:themeTint` / `w:themeShade` applied as
/// Word draws it, upper-case; with both, the tint alone counts. `None` for a base that is
/// not six hex digits.
pub fn word_theme_tint(base: &str, tint: Option<&str>, shade: Option<&str>) -> Option<String> {
    let base = base.trim_start_matches('#').to_uppercase();
    let channel = |k: usize| {
        base.get(k..k + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map(i64::from)
    };
    let (r, g, b) = (channel(0)?, channel(2)?, channel(4)?);
    let tinting = hex_byte(tint).is_some();
    let amount = if tinting { hex_byte(tint) } else { hex_byte(shade) };
    let Some(amount) = amount.filter(|a| *a != 255) else {
        return Some(base);
    };
    let (high, low) = (r.max(g).max(b), r.min(g).min(b));
    let lum = 3 * (high + low);
    let lum2 = if tinting {
        floor_div(lum * amount + SCALE * (255 - amount), 255)
    } else {
        floor_div(lum * amount, 255)
    };
    if high == low {
        let grey = floor_div(lum2 * 255, SCALE);
        return Some(format!("{grey:02X}{grey:02X}{grey:02X}"));
    }
    let span = high - low;
    let sat = if high + low <= 255 {
        floor_div(span * SCALE, high + low)
    } else {
        floor_div(span * SCALE, 510 - high - low)
    };
    let term = |c: i64| -floor_div(-(high - c) * SECTOR, span);
    let (rd, gd, bd) = (term(r), term(g), term(b));
    let hue = if r == high {
        bd - gd
    } else if g == high {
        SCALE / 3 + rd - bd
    } else {
        2 * SCALE / 3 + gd - rd
    }
    .rem_euclid(SCALE);
    let m2 = if 2 * lum2 <= SCALE {
        floor_div(lum2 * (SCALE + sat), SCALE)
    } else {
        lum2 + sat - floor_div(lum2 * sat, SCALE)
    };
    let m1 = 2 * lum2 - m2;
    let ramp = |h: i64| {
        let h = h.rem_euclid(SCALE);
        let v = if h < SECTOR {
            m1 + floor_div((m2 - m1) * h, SECTOR)
        } else if h < SCALE / 2 {
            m2
        } else if h < 2 * SCALE / 3 {
            m1 + floor_div((m2 - m1) * (2 * SCALE / 3 - h), SECTOR)
        } else {
            m1
        };
        floor_div(v * 255, SCALE).clamp(0, 255)
    };
    Some(format!(
        "{:02X}{:02X}{:02X}",
        ramp(hue + SCALE / 3),
        ramp(hue),
        ramp(hue - SCALE / 3)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tints_and_shades_as_word_draws_them() {
        assert_eq!(word_theme_tint("4472C4", None, None).as_deref(), Some("4472C4"));
        assert_eq!(word_theme_tint("4472C4", None, Some("FF")).as_deref(), Some("4472C4"));
        assert_eq!(word_theme_tint("808080", Some("00"), None).as_deref(), Some("FFFFFF"));
        assert_eq!(word_theme_tint("808080", None, Some("00")).as_deref(), Some("000000"));
        assert_eq!(word_theme_tint("zz", None, None), None);
    }
}
