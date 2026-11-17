//! Owns: text in a Microsoft symbol face drawn without the face (ooxml-common's
//! `text.symbol_fonts`): which families are symbol faces ([`symbol_face`]), what each of
//! their codes stands for in Unicode ([`to_unicode`], from `data/symbol-maps.tsv`, which
//! `data/generate.py` writes from ooxml-common's tables: Unicode's own identification of
//! every glyph, under the measured drawing preferences), a text translated
//! ([`translate`]), and the families that draw the result ([`FALLBACK_FAMILIES`]).
//!
//! Office writes a symbol either as the code itself (0x20-0xFF, PowerPoint's bullets and
//! runs) or as its private-use alias U+F020-U+F0FF (Word's `w:sym` and bullets); both mean
//! the same glyph.
//!
//! Must not: decide whether a face is drawn this way (a renderer maps only a face the
//! drawing has no copy of), or hold a table written by hand.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Faces that draw what [`to_unicode`] returns, best first: Windows', macOS' (Apple
/// Symbols, and Zapf Dingbats for the Dingbats block), then the open families a server is
/// likely to have (ooxml-common's `FALLBACK_FAMILIES`).
pub const FALLBACK_FAMILIES: [&str; 7] = [
    "Segoe UI Symbol",
    "Apple Symbols",
    "Zapf Dingbats",
    "Noto Sans Symbols 2",
    "Noto Sans Symbols",
    "DejaVu Sans",
    "Symbola",
];

#[derive(Default)]
struct Tables {
    /// A face's key (no spaces, lower case) to its table's name.
    faces: HashMap<String, String>,
    /// A table's code to code point.
    maps: HashMap<String, HashMap<u32, u32>>,
    /// A table's code to the text drawn in its place, where preferred.
    preferred: HashMap<String, HashMap<u32, String>>,
}

fn hex(text: &str) -> Option<u32> {
    u32::from_str_radix(text, 16).ok()
}

fn pairs(field: &str) -> impl Iterator<Item = (u32, &str)> {
    field.split(' ').filter_map(|pair| {
        let (code, value) = pair.split_once(':')?;
        Some((hex(code)?, value))
    })
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut out = Tables::default();
        for line in include_str!("../data/symbol-maps.tsv").lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            let [kind, name, value] = fields.as_slice() else {
                continue;
            };
            match *kind {
                "face" => {
                    out.faces.insert((*name).to_owned(), (*value).to_owned());
                }
                "map" => {
                    let map = pairs(value).filter_map(|(c, p)| Some((c, hex(p)?))).collect();
                    out.maps.insert((*name).to_owned(), map);
                }
                "preferred" => {
                    let map = pairs(value)
                        .filter_map(|(c, points)| {
                            let text: Option<String> =
                                points.split('+').map(|p| hex(p).and_then(char::from_u32)).collect();
                            Some((c, text?))
                        })
                        .collect();
                    out.preferred.insert((*name).to_owned(), map);
                }
                _ => {}
            }
        }
        out
    })
}

/// The table name of `family` when it names a symbol face (`"Wingdings 2"` is
/// `"wingdings 2"`), else `None`.
pub fn symbol_face(family: &str) -> Option<&'static str> {
    let key: String = family.split_whitespace().collect::<String>().to_lowercase();
    tables().faces.get(&key).map(String::as_str)
}

/// The face's code (0x20-0xFF) `ch` stands for: itself, or its private-use alias.
pub fn code_of(ch: char) -> Option<u32> {
    let mut point = u32::from(ch);
    if (0xF020..=0xF0FF).contains(&point) {
        point -= 0xF000;
    }
    (0x20..=0xFF).contains(&point).then_some(point)
}

/// What `ch` in symbol face `face` (a [`symbol_face`] name) is, as Unicode text, or `None`
/// where the glyph has no equivalent.
pub fn to_unicode(face: &str, ch: char) -> Option<String> {
    let code = code_of(ch)?;
    let tables = tables();
    if let Some(text) = tables.preferred.get(face).and_then(|m| m.get(&code)) {
        return Some(text.clone());
    }
    let point = tables.maps.get(face)?.get(&code)?;
    char::from_u32(*point).map(String::from)
}

/// `text` in `face` as the Unicode it stands for, and the characters that have no
/// equivalent (left as they are). Characters outside the face's codes pass through.
pub fn translate(face: &str, text: &str) -> (String, String) {
    let mut out = String::with_capacity(text.len());
    let mut unmapped = String::new();
    for ch in text.chars() {
        if ch == '\n' || ch == '\t' {
            out.push(ch);
            continue;
        }
        match to_unicode(face, ch) {
            Some(mapped) => out.push_str(&mapped),
            None => {
                if code_of(ch).is_some() && ch != ' ' {
                    unmapped.push(ch);
                }
                out.push(ch);
            }
        }
    }
    (out, unmapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_symbol_face_is_found_without_spaces_or_case() {
        assert_eq!(symbol_face("Wingdings 2"), Some("wingdings 2"));
        assert_eq!(symbol_face("SYMBOL"), Some("symbol"));
        assert_eq!(symbol_face("Calibri"), None);
    }

    #[test]
    fn codes_and_their_aliases_mean_one_glyph() {
        assert_eq!(to_unicode("wingdings", 'l').as_deref(), Some("\u{25cf}"));
        assert_eq!(to_unicode("wingdings", '\u{f06c}').as_deref(), Some("\u{25cf}"));
        assert_eq!(to_unicode("symbol", 'a').as_deref(), Some("\u{3b1}"));
        let (text, unmapped) = translate("wingdings", "\u{fc}\t");
        assert_eq!((text.as_str(), unmapped.as_str()), ("\u{2713}\t", ""));
    }
}
