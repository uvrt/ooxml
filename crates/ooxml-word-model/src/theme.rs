//! Owns: what the model reads of the theme part -- its font scheme ([`FontScheme`]) and colour
//! scheme -- and how WordprocessingML names a theme colour ([`ThemeColours`]): `w:themeColor`
//! / `w:themeFill` through `w:clrSchemeMapping`, with Word's own tint and shade arithmetic
//! ([`word_theme_tint`], measured in Word 16.106, is the colour engine's: `ooxml-colour`).
//! Word draws the theme colour, not the `w:val` beside it.
//!
//! Must not: implement DrawingML colour transforms (`lumMod`, `lumOff`: phase P3's colour
//! engine); these attributes are not DrawingML transforms.

use crate::value::{text, Text};
use crate::w;
use crate::xmlread::{attr, elements, local};
pub use ooxml_colour::word_theme_tint;
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
