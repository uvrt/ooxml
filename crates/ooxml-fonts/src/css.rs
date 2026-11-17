//! Owns: the `font-family` stack a drawing names a face with (ooxml-common's
//! `fontmap.font_family_value`): the names asked for, each followed by the typographic
//! family a rasteriser files it under, its substitute under the font mapping, and the face
//! it is measured with, then the generic family -- each name once.
//!
//! Must not: look a face up on disk, or decide what a face is measured with (the
//! substitution table does).

use crate::substitutes::{family_key, generic_family, office_substitutions, substitution_for};

/// Style words an Office family name appends to its typographic family.
const TYPOGRAPHIC_STYLE_WORDS: [&str; 17] = [
    "black",
    "bold",
    "book",
    "demibold",
    "display",
    "extrabold",
    "extralight",
    "hairline",
    "heavy",
    "light",
    "medium",
    "regular",
    "semibold",
    "semilight",
    "thin",
    "ultrabold",
    "ultralight",
];

/// The family mapping a stack substitutes faces with: requested family to drawn family, in
/// order (the first of two names with one key wins a case-insensitive lookup).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontMapping {
    pairs: Vec<(String, String)>,
}

impl FontMapping {
    /// The default: every Office face that is drawn with another (ooxml-common's
    /// `DEFAULT_FONT_MAPPING`).
    pub fn office_default() -> FontMapping {
        FontMapping {
            pairs: office_substitutions()
                .iter()
                .filter(|row| !row.exact())
                .map(|row| (row.office.clone(), row.substitute.clone()))
                .collect(),
        }
    }

    /// A mapping of these pairs, in order (a later pair with the same name replaces an
    /// earlier one in place, as a dictionary does).
    pub fn new(pairs: impl IntoIterator<Item = (String, String)>) -> FontMapping {
        let mut out: Vec<(String, String)> = Vec::new();
        for (k, v) in pairs {
            match out.iter_mut().find(|(name, _)| *name == k) {
                Some(slot) => slot.1 = v,
                None => out.push((k, v)),
            }
        }
        FontMapping { pairs: out }
    }

    /// The family `family` is drawn with: the mapping's (exact name, then full-width forms
    /// normalised, then case-insensitively), else the substitution table's.
    pub fn mapped(&self, family: &str) -> Option<String> {
        if family.is_empty() {
            return None;
        }
        if let Some((_, v)) = self.pairs.iter().find(|(k, _)| k == family) {
            return Some(v.clone());
        }
        let normalized = half_width(family);
        if normalized != family {
            if let Some((_, v)) = self.pairs.iter().find(|(k, _)| *k == normalized) {
                return Some(v.clone());
            }
        }
        let lowered = normalized.trim().to_lowercase();
        if let Some((_, v)) = self.pairs.iter().find(|(k, _)| family_key(k) == lowered) {
            return Some(v.clone());
        }
        substitution_for(family).map(|row| row.substitute.clone())
    }
}

fn half_width(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(u32::from(c) - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// The family name a rasteriser indexes `family` under when it differs: one trailing style
/// word stripped (`Aptos Display` -> `Aptos`).
pub fn typographic_family(family: &str) -> Option<&str> {
    let (head, tail) = family.rsplit_once(' ')?;
    if head.is_empty() || !TYPOGRAPHIC_STYLE_WORDS.contains(&tail.to_lowercase().as_str()) {
        return None;
    }
    Some(head)
}

fn escape(name: &str) -> String {
    name.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The `font-family` stack for `fonts` (empty names and theme pointers left out), or `None`
/// when none is left: names with a space quoted, entities escaped, the first name's
/// generic family last.
pub fn font_family_value(fonts: &[&str], mapping: &FontMapping) -> Option<String> {
    let mut unique: Vec<String> = Vec::new();
    let add = |name: String, unique: &mut Vec<String>| {
        if !unique.contains(&name) {
            unique.push(name);
        }
    };
    for &font in fonts {
        if font.is_empty() || unique.iter().any(|u| u == font) || font.starts_with('+') {
            continue;
        }
        add(font.to_owned(), &mut unique);
        if let Some(t) = typographic_family(font) {
            add(t.to_owned(), &mut unique);
        }
        if let Some(s) = mapping.mapped(font) {
            add(s, &mut unique);
        }
        if let Some(row) = substitution_for(font) {
            add(row.substitute.clone(), &mut unique);
        }
    }
    let first = unique.first()?;
    let generic = generic_family(first).name();
    let mut parts: Vec<String> = unique
        .iter()
        .map(|name| {
            if name.contains(' ') {
                format!("'{}'", escape(name))
            } else {
                escape(name)
            }
        })
        .collect();
    parts.push(generic.to_owned());
    Some(parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacks() {
        let m = FontMapping::office_default();
        assert_eq!(
            font_family_value(&["Arial"], &m).as_deref(),
            Some("Arial, Arimo, sans-serif")
        );
        assert_eq!(
            font_family_value(&["Aptos Display"], &m).as_deref(),
            Some("'Aptos Display', Aptos, Carlito, sans-serif")
        );
        assert_eq!(font_family_value(&["+mn-lt"], &m), None);
        assert_eq!(typographic_family("Calibri Light"), Some("Calibri"));
    }
}
