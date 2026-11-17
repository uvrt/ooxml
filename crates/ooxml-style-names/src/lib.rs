//! Word's built-in style names in the languages Word speaks, and a field's style name found
//! by them (docs/architecture/README.md, section 3; layer 1): ooxml-common's `stylenames`,
//! its measured table (`stylenames.json`) written into `data/style-names.tsv` by
//! `data/generate.py`.
//!
//! A field that names a style -- a table of contents' `\t "Kop 2;1"`, `STYLEREF "Titel"`
//! -- names it as the Word that wrote it shows it. `styles.xml` stores a built-in style
//! under its English name (`heading 2`) whatever the language, and Word matches the field's
//! name against its interface's display names. So a name is accepted in the document's
//! language and in English: first a style's stored name or alias, then the built-in style
//! the document's language displays under that name, then the one English Word displays
//! under it -- case ignored, and trimmed where the field's Word trims ([`find_style`]).
//!
//! Owns: the measured table ([`builtin_names`], [`list_separator`], [`measured_language`])
//! and the lookup.
//!
//! Must not: read a document or hold state: the table is parsed once into an immutable
//! value, `Send + Sync`, and nothing here depends on an interpreter lock.
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::OnceLock;

/// One measured language: its region, its list separator and its built-in styles' names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    /// Its code (`en`, `nl`, ...).
    pub code: String,
    /// The region it was measured in (`nl-NL`).
    pub region: String,
    /// The list separator of that region, which a `\t` list is split on.
    pub list_separator: String,
    /// Each built-in style's stored name (`heading 2`) and the name that language's Word
    /// shows (`Kop 2`), in the table's order.
    pub names: Vec<(String, String)>,
    /// Display name (folded) -> stored name (folded); a later row wins (`_reverse`).
    reverse: HashMap<String, String>,
}

/// The measured languages, in the table's order.
pub fn languages() -> &'static [Language] {
    static TABLE: OnceLock<Vec<Language>> = OnceLock::new();
    TABLE.get_or_init(|| parse(include_str!("../data/style-names.tsv")))
}

fn parse(text: &str) -> Vec<Language> {
    let mut out: Vec<Language> = Vec::new();
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["language", code, region, separator] => out.push(Language {
                code: (*code).to_owned(),
                region: (*region).to_owned(),
                list_separator: (*separator).to_owned(),
                names: Vec::new(),
                reverse: HashMap::new(),
            }),
            ["name", stored, display] => {
                if let Some(language) = out.last_mut() {
                    language.names.push(((*stored).to_owned(), (*display).to_owned()));
                    language.reverse.insert(fold(display.trim()), fold(stored));
                }
            }
            _ => {}
        }
    }
    out
}

/// A name without regard to case, as Python's `str.casefold` folds the names measured:
/// lower case, with `ß` as `ss` and a final sigma as a sigma.
pub fn fold(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            'ß' | 'ẞ' => out.push_str("ss"),
            'ς' => out.push('σ'),
            other => out.extend(other.to_lowercase()),
        }
    }
    out
}

/// The measured language a BCP 47 tag (`nl-BE`, `fr_FR`, `de`) falls under -- its primary
/// subtag, lower case -- or `None` when that language was not measured.
pub fn measured_language(tag: Option<&str>) -> Option<&'static Language> {
    let tag = tag.filter(|t| !t.is_empty())?;
    let normal = tag.replace('_', "-");
    let primary = normal.split('-').next().unwrap_or_default().to_lowercase();
    languages().iter().find(|l| l.code == primary)
}

/// Stored name -> display name of every built-in style in the language of `tag` (`nl-NL`
/// is `nl`); empty for a language not measured.
pub fn builtin_names(tag: Option<&str>) -> &'static [(String, String)] {
    measured_language(tag).map_or(&[], |l| l.names.as_slice())
}

/// The list separator Word splits a field's list on in the region of `tag`'s language, as
/// measured; `default` for a language not measured.
pub fn list_separator<'d>(tag: Option<&str>, default: &'d str) -> &'d str {
    match measured_language(tag) {
        Some(l) => l.list_separator.as_str(),
        None => default,
    }
}

/// The style a field names (`find_style`): `name` as the field gives it, `styles` each
/// style's stored name -- and each of its aliases -- with what the caller wants back (its
/// id), `language` the document's tag (`None`: English only). In order: a stored name or
/// alias (the first of a name winning); the built-in style `language`'s Word shows as
/// `name`; the one English Word shows as `name`. Case-insensitive; `trim` -- the name taken
/// without the spaces round it -- is the field's to say (`TOC \t` trims, `STYLEREF` does
/// not). `None` when nothing matches.
pub fn find_style<V: Clone>(name: &str, styles: &[(&str, V)], language: Option<&str>, trim: bool) -> Option<V> {
    let wanted = fold(if trim { name.trim() } else { name });
    if wanted.trim().is_empty() {
        return None;
    }
    let mut stored: HashMap<String, V> = HashMap::new();
    for (key, value) in styles.iter().rev() {
        stored.insert(fold(key.trim()), value.clone());
    }
    if let Some(found) = stored.get(&wanted) {
        return Some(found.clone());
    }
    let own = measured_language(language).map(|l| l.code.as_str());
    let mut order: Vec<&str> = own.into_iter().collect();
    if !order.contains(&"en") {
        order.push("en");
    }
    for code in order {
        let Some(language) = languages().iter().find(|l| l.code == code) else {
            continue;
        };
        if let Some(found) = language
            .reverse
            .get(&wanted)
            .and_then(|stored_name| stored.get(stored_name))
        {
            return Some(found.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_found_stored_then_by_the_language_then_in_english() {
        let styles = [
            ("heading 2", "Heading2"),
            ("Chapter title", "Chaptertitle"),
            ("Title", "Title"),
        ];
        assert_eq!(find_style("HEADING 2", &styles, None, false), Some("Heading2"));
        assert_eq!(find_style("Kop 2", &styles, Some("nl-NL"), false), Some("Heading2"));
        assert_eq!(find_style("Kop 2", &styles, None, false), None);
        assert_eq!(find_style(" Chapter title ", &styles, None, false), None);
        assert_eq!(find_style(" Chapter title ", &styles, None, true), Some("Chaptertitle"));
        assert_eq!(find_style("Titel", &styles, Some("nl"), false), Some("Title"));
        assert_eq!(find_style("  ", &styles, None, true), None);
    }

    #[test]
    fn languages_and_separators() {
        assert_eq!(measured_language(Some("nl_BE")).map(|l| l.code.as_str()), Some("nl"));
        assert!(measured_language(Some("xx")).is_none());
        assert_eq!(list_separator(Some("nl-NL"), ","), ";");
        assert_eq!(list_separator(Some("en-US"), ";"), ",");
        assert_eq!(list_separator(None, ","), ",");
        assert!(builtin_names(Some("de")).iter().any(|(s, _)| s == "heading 1"));
        assert_eq!(fold("Grußformel"), "grussformel");
    }
}
