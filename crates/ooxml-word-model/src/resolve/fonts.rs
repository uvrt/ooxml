//! Owns: which face, size and weight a run's characters are measured in -- docx2svg's
//! `resolve.fonts` (measured, `make_style_probe.py` cases f..): a character's slot
//! ([`slot_for`]: `w:cs`/`w:rtl` runs take `cs`; otherwise by Unicode range, with
//! `w:hint="eastAsia"`), the face a slot names (directly or through the theme, East Asian
//! and complex-script references only through `w:themeFontLang`), and the run's format in
//! each slot ([`CharacterFormat`]).
//!
//! Must not: substitute a face the machine lacks or read font files (`ooxml-fonts`, L1).

use crate::settings::Settings;
use crate::theme::FontScheme;
use crate::value::{text, FontRef, Props, Text, Value};

/// `w:sz` when nothing declares one: 12 pt, as Word draws it.
pub const APPLICATION_HALF_POINTS: i64 = 24;

/// A `w:rFonts` slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Slot {
    /// `ascii`.
    Ascii,
    /// `hAnsi`.
    HAnsi,
    /// `eastAsia`.
    EastAsia,
    /// `cs`.
    ComplexScript,
}

impl Slot {
    /// Its name in `w:rFonts`.
    pub fn name(self) -> &'static str {
        match self {
            Slot::Ascii => "ascii",
            Slot::HAnsi => "hAnsi",
            Slot::EastAsia => "eastAsia",
            Slot::ComplexScript => "cs",
        }
    }

    /// The face Word draws when the slot names none.
    fn fallback(self) -> &'static str {
        match self {
            Slot::EastAsia => "MS Mincho",
            _ => "Times New Roman",
        }
    }

    /// The theme reference Word uses where nothing names the slot.
    fn default_theme(self) -> &'static str {
        match self {
            Slot::Ascii | Slot::HAnsi => "minorHAnsi",
            Slot::EastAsia => "minorEastAsia",
            Slot::ComplexScript => "minorBidi",
        }
    }
}

const EAST_ASIAN_RANGES: &[(u32, u32)] = &[
    (0x1100, 0x11FF),
    (0x2E80, 0x2FDF),
    (0x2FF0, 0x31FF),
    (0x3200, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0xA000, 0xA4CF),
    (0xAC00, 0xD7AF),
    (0xF900, 0xFAFF),
    (0xFE30, 0xFE4F),
    (0xFF00, 0xFFEF),
    (0x20000, 0x2FFFF),
];

fn latin1_hinted(code: u32) -> bool {
    matches!(code, 0xA1 | 0xA4 | 0xA7 | 0xA8 | 0xAA | 0xAD | 0xAF | 0xD7 | 0xF7)
        || (0xB0..0xB5).contains(&code)
        || (0xB6..0xBB).contains(&code)
        || (0xBC..0xC0).contains(&code)
}

fn latin1_hinted_chinese(code: u32) -> bool {
    matches!(
        code,
        0xE0 | 0xE1 | 0xE8 | 0xE9 | 0xEA | 0xEC | 0xED | 0xF2 | 0xF3 | 0xF9 | 0xFA | 0xFC
    )
}

/// The slot one character of a run takes.
pub fn slot_for(c: char, complex_script: bool, hint: Option<&str>, east_asian_language: Option<&str>) -> Slot {
    if complex_script {
        return Slot::ComplexScript;
    }
    let code = u32::from(c);
    if code < 0x80 {
        return Slot::Ascii;
    }
    if EAST_ASIAN_RANGES
        .iter()
        .any(|(low, high)| (*low..=*high).contains(&code))
    {
        return Slot::EastAsia;
    }
    if hint == Some("eastAsia") {
        if (0xA0..=0xFF).contains(&code) {
            if latin1_hinted(code) {
                return Slot::EastAsia;
            }
            let chinese = east_asian_language.unwrap_or("").to_lowercase().starts_with("zh");
            return if chinese && latin1_hinted_chinese(code) {
                Slot::EastAsia
            } else {
                Slot::HAnsi
            };
        }
        return Slot::EastAsia;
    }
    Slot::HAnsi
}

fn script_of_language(tag: &str) -> Option<&'static str> {
    let tag = tag.to_lowercase();
    let lookup = |t: &str| -> Option<&'static str> {
        Some(match t {
            "he" | "yi" => "Hebr",
            "ar" | "fa" | "ur" | "ps" | "sd" | "ug" => "Arab",
            "syr" => "Syrc",
            "dv" => "Thaa",
            "ja" => "Jpan",
            "ko" => "Hang",
            "zh-cn" | "zh-sg" | "zh" => "Hans",
            "zh-tw" | "zh-hk" | "zh-mo" => "Hant",
            "th" => "Thai",
            "hi" | "mr" | "ne" | "sa" => "Deva",
            "bn" | "as" => "Beng",
            "ta" => "Taml",
            "te" => "Telu",
            "kn" => "Knda",
            "ml" => "Mlym",
            "gu" => "Gujr",
            "pa" => "Guru",
            "or" => "Orya",
            "km" => "Khmr",
            "lo" => "Laoo",
            "my" => "Mymr",
            "si" => "Sinh",
            "bo" => "Tibt",
            "am" | "ti" => "Ethi",
            "iu" => "Cans",
            "chr" => "Cher",
            "mn" => "Mong",
            "ka" => "Geor",
            "hy" => "Armn",
            "vi" => "Viet",
            _ => return None,
        })
    };
    lookup(&tag).or_else(|| lookup(tag.split('-').next().unwrap_or("")))
}

/// The face a theme reference (`minorHAnsi`, `majorBidi`...) names, or `None` where Word
/// does not follow it.
pub fn theme_face(reference: &str, scheme: Option<&FontScheme>, settings: &Settings) -> Option<Text> {
    let builtin;
    let scheme = match scheme {
        Some(s) => s,
        None => {
            builtin = FontScheme::builtin();
            &builtin
        }
    };
    let fonts = if reference.starts_with("major") {
        &scheme.major
    } else {
        &scheme.minor
    };
    if reference.ends_with("Ascii") || reference.ends_with("HAnsi") {
        return (!fonts.latin.is_empty()).then(|| fonts.latin.clone());
    }
    let east_asian = reference.ends_with("EastAsia");
    let language = settings
        .theme_font_lang
        .get(if east_asian { "eastAsia" } else { "bidi" })?;
    if let Some(face) = script_of_language(language)
        .and_then(|s| fonts.scripts.get(s))
        .filter(|f| !f.is_empty())
    {
        return Some(face.clone());
    }
    let face = if east_asian {
        &fonts.east_asian
    } else {
        &fonts.complex_script
    };
    (!face.is_empty()).then(|| face.clone())
}

/// What one slot of a run is measured in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterFormat {
    /// The slot.
    pub slot: Slot,
    /// The face (Word's fallback where the slot names none).
    pub face: Text,
    /// The size, half points (`w:sz`, or `w:szCs` in a complex-script run).
    pub half_points: i64,
    /// Bold (`w:b`, or `w:bCs`).
    pub bold: bool,
    /// Italic (`w:i`, or `w:iCs`).
    pub italic: bool,
    /// `superscript` or `subscript`.
    pub vertical_align: Option<Text>,
    /// `w:position`, half points.
    pub position: i64,
    /// `w:bdr`'s (width in eighths of a point, space in points), where it has one.
    pub border: Option<(i64, i64)>,
}

/// Whether a run's characters all take the complex-script slot (`w:cs` or `w:rtl`).
pub fn complex_script(resolved: &Props) -> bool {
    resolved.get("cs").is_some_and(Value::truthy) || resolved.get("rtl").is_some_and(Value::truthy)
}

/// The run's format in `slot`.
pub fn character_format(
    resolved: &Props,
    slot: Slot,
    scheme: Option<&FontScheme>,
    settings: &Settings,
) -> CharacterFormat {
    let reference = match resolved.get(format!("rFonts.{}", slot.name()).as_str()) {
        Some(Value::Font(f)) => f.clone(),
        _ => FontRef {
            face: None,
            theme: Some(text(slot.default_theme())),
        },
    };
    let face = match reference.theme.as_deref().filter(|t| !t.is_empty()) {
        Some(theme) => theme_face(theme, scheme, settings),
        None => reference.face.clone(),
    };
    let complex = complex_script(resolved);
    let (size_key, bold_key, italic_key) = if complex {
        ("szCs", "bCs", "iCs")
    } else {
        ("sz", "b", "i")
    };
    let vertical = resolved
        .get("vertAlign")
        .and_then(Value::str)
        .filter(|v| matches!(*v, "superscript" | "subscript"));
    let border = match resolved.get("bdr") {
        Some(Value::RunBorder(b)) if !matches!(b.style.as_deref(), None | Some("none" | "nil")) => {
            Some((b.size, b.space))
        }
        _ => None,
    };
    CharacterFormat {
        slot,
        face: face.unwrap_or_else(|| text(slot.fallback())),
        half_points: resolved
            .get(size_key)
            .and_then(Value::int)
            .unwrap_or(APPLICATION_HALF_POINTS),
        bold: resolved.get(bold_key).is_some_and(Value::truthy),
        italic: resolved.get(italic_key).is_some_and(Value::truthy),
        vertical_align: vertical.map(text),
        position: resolved.get("position").and_then(Value::int).unwrap_or(0),
        border,
    }
}
