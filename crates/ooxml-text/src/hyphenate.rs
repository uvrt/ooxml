//! Owns: where a word may break, by language -- Liang's method over the openly licensed
//! patterns docx2svg ships (hyph-utf8; their sources and licences are in
//! `python/docx2svg/src/docx2svg/patterns/SOURCES.txt`), after the word lists a language
//! consults first (English: the probe's observed exceptions, then the Moby list). A port
//! of docx2svg's `hyphenate`, reading the very same files.
//!
//! Must not: decide which points a line takes (that is the `auto` module and
//! [`crate::breaks`]), or hold anything but the tables parsed once from the files compiled
//! in.

use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

macro_rules! patterns_file {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../python/docx2svg/src/docx2svg/patterns/",
            $name
        ))
    };
}

/// A word list's break mark.
const WORD_MARK: char = '=';
/// Word's shortest fragment before a break, every language and mode.
pub const LEFT_MIN: usize = 2;

/// The pattern sets, by name: patterns and exceptions.
fn pattern_files(name: &str) -> Option<(&'static [u8], &'static [u8])> {
    Some(match name {
        "en-us-knuth" => (
            patterns_file!("hyph-en-us-knuth.pat.txt"),
            patterns_file!("hyph-en-us-knuth.hyp.txt"),
        ),
        "nl" => (patterns_file!("hyph-nl.pat.txt"), patterns_file!("hyph-nl.hyp.txt")),
        "de-1996" => (patterns_file!("hyph-de-1996.pat.txt"), b""),
        "fr" => (patterns_file!("hyph-fr.pat.txt"), b""),
        _ => return None,
    })
}

/// `w:lang` (lowercased) -> the pattern set Word's hyphenator for it is modelled by;
/// `None`: not hyphenated (measured: nl-BE and fr-BE are not).
const PATTERN_SETS: [(&str, Option<&str>); 6] = [
    ("en", Some("en-us-knuth")),
    ("nl", Some("nl")),
    ("nl-be", None),
    ("de", Some("de-1996")),
    ("fr", Some("fr")),
    ("fr-be", None),
];

/// The pattern set for a `w:lang` value, by the whole tag and then its primary subtag.
pub fn pattern_set(lang: Option<&str>) -> Option<&'static str> {
    let tag = lang.filter(|l| !l.is_empty())?.to_lowercase();
    let find = |key: &str| PATTERN_SETS.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
    find(&tag).unwrap_or_else(|| find(tag.split('-').next().unwrap_or_default()).flatten())
}

/// One language's patterns and exceptions.
struct Patterns {
    table: HashMap<String, Vec<u8>>,
    longest: usize,
    exceptions: HashMap<String, Vec<usize>>,
}

fn lines(bytes: &[u8]) -> impl Iterator<Item = &str> {
    std::str::from_utf8(bytes)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
}

fn positions(spelled: &str, mark: char) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = 0;
    for c in spelled.chars() {
        if c == mark {
            out.push(at);
        } else {
            at += 1;
        }
    }
    out
}

impl Patterns {
    fn parse(patterns: &[u8], exceptions: &[u8]) -> Patterns {
        let mut table = HashMap::new();
        let mut longest = 0;
        for pattern in lines(patterns) {
            let letters: String = pattern.chars().filter(|c| !c.is_ascii_digit()).collect();
            let count = letters.chars().count();
            let mut values = vec![0u8; count + 1];
            let mut at = 0;
            for c in pattern.chars() {
                match c.to_digit(10) {
                    Some(d) => {
                        if let Some(slot) = values.get_mut(at) {
                            *slot = d as u8;
                        }
                    }
                    None => at += 1,
                }
            }
            longest = longest.max(count);
            table.insert(letters, values);
        }
        let exceptions = lines(exceptions)
            .map(|item| (item.replace('-', ""), positions(item, '-')))
            .collect();
        Patterns {
            table,
            longest,
            exceptions,
        }
    }

    /// The value before every letter of `word` (lower case) and after its last.
    fn values(&self, word: &[char]) -> Vec<u8> {
        let mut text = Vec::with_capacity(word.len() + 2);
        text.push('.');
        text.extend_from_slice(word);
        text.push('.');
        let mut values = vec![0u8; text.len() + 1];
        let mut key = String::new();
        for i in 0..text.len() {
            key.clear();
            for j in i + 1..=text.len().min(i + self.longest) {
                if let Some(&c) = text.get(j - 1) {
                    key.push(c);
                }
                let Some(found) = self.table.get(&key) else { continue };
                for (k, value) in found.iter().enumerate() {
                    if let Some(slot) = values.get_mut(i + k) {
                        *slot = (*slot).max(*value);
                    }
                }
            }
        }
        values
            .get(1..values.len().saturating_sub(1))
            .unwrap_or_default()
            .to_vec()
    }

    /// Every `i` (1 to len - 1) after which `word` may break, before minimums.
    fn points(&self, word: &str) -> Vec<usize> {
        let lower = word.to_lowercase();
        if let Some(found) = self.exceptions.get(&lower) {
            return found.clone();
        }
        let chars: Vec<char> = lower.chars().collect();
        let values = self.values(&chars);
        (1..word.chars().count())
            .filter(|i| values.get(*i).is_some_and(|v| v % 2 == 1))
            .collect()
    }
}

fn patterns(name: &str) -> Option<&'static Patterns> {
    static SETS: OnceLock<HashMap<&'static str, Patterns>> = OnceLock::new();
    let sets = SETS.get_or_init(|| {
        ["en-us-knuth", "nl", "de-1996", "fr"]
            .into_iter()
            .filter_map(|name| pattern_files(name).map(|(p, e)| (name, Patterns::parse(p, e))))
            .collect()
    });
    sets.get(name)
}

/// Python's `str.casefold` for the characters the word lists hold: lower case, with the
/// sharp s, long s, final sigma and Latin ligatures folded.
fn casefold(word: &str) -> String {
    word.chars()
        .flat_map(char::to_lowercase)
        .flat_map(|c| -> Vec<char> {
            match c {
                'ß' | 'ẞ' => vec!['s', 's'],
                'ſ' => vec!['s'],
                'ς' => vec!['σ'],
                'ﬀ' => vec!['f', 'f'],
                'ﬁ' => vec!['f', 'i'],
                'ﬂ' => vec!['f', 'l'],
                'ﬃ' => vec!['f', 'f', 'i'],
                'ﬄ' => vec!['f', 'f', 'l'],
                'ﬅ' | 'ﬆ' => vec!['s', 't'],
                other => vec![other],
            }
        })
        .collect()
}

/// Words with their break points, by their case-folded spelling.
struct WordList {
    words: HashMap<String, Vec<String>>,
}

impl WordList {
    fn parse(text: &str) -> WordList {
        let mut words: HashMap<String, Vec<String>> = HashMap::new();
        for entry in text.lines().filter(|l| !l.is_empty()) {
            let spelling = entry.split(' ').next().unwrap_or_default().replace(WORD_MARK, "");
            words.entry(casefold(&spelling)).or_default().push(entry.to_owned());
        }
        WordList { words }
    }

    /// Where the list breaks `word` (as typed first, then lower case, then the first
    /// spelling it holds), or `None` where it does not hold it.
    fn points(&self, word: &str, mode15: bool) -> Option<Vec<usize>> {
        let found = self.words.get(&casefold(word))?;
        let spelling = |entry: &String| entry.split(' ').next().unwrap_or_default().replace(WORD_MARK, "");
        let first = |wanted: &str| found.iter().find(|e| spelling(e) == wanted);
        let entry = first(word)
            .or_else(|| first(&word.to_lowercase()))
            .or_else(|| found.first())?;
        let parts: Vec<&str> = entry.split(' ').collect();
        let chosen = if mode15 { parts.last() } else { parts.first() };
        Some(positions(chosen.copied().unwrap_or_default(), WORD_MARK))
    }
}

fn word_lists(name: &str) -> &'static [WordList] {
    static LISTS: OnceLock<Vec<WordList>> = OnceLock::new();
    if name != "en-us-knuth" {
        return &[];
    }
    LISTS.get_or_init(|| {
        let observed = std::str::from_utf8(patterns_file!("en-observed-exceptions.txt")).unwrap_or_default();
        let mut moby = String::new();
        let _ = GzDecoder::new(patterns_file!("en-moby.txt.gz").as_slice()).read_to_string(&mut moby);
        vec![WordList::parse(observed), WordList::parse(&moby)]
    })
}

/// Every `i` after which `word` may break by pattern set `name` and the word lists before
/// it, before minimums.
fn word_points(word: &str, name: &str, mode15: bool) -> Vec<usize> {
    for list in word_lists(name) {
        if let Some(found) = list.points(word, mode15) {
            return found;
        }
    }
    patterns(name).map(|p| p.points(word)).unwrap_or_default()
}

/// Where `word` (letters only) may break in language `lang`: every `i` such that its first
/// `i` letters and the rest may end and start a line, at least [`LEFT_MIN`] and
/// `right_min` letters long.
pub fn points(word: &str, lang: Option<&str>, right_min: usize, mode15: bool) -> Vec<usize> {
    let Some(name) = pattern_set(lang) else {
        return Vec::new();
    };
    let length = word.chars().count();
    word_points(word, name, mode15)
        .into_iter()
        .filter(|i| LEFT_MIN <= *i && *i + right_min <= length)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages() {
        assert_eq!(pattern_set(Some("en-GB")), Some("en-us-knuth"));
        assert_eq!(pattern_set(Some("nl-BE")), None);
        assert_eq!(pattern_set(Some("de-CH")), Some("de-1996"));
        assert_eq!(pattern_set(Some("x-none")), None);
        assert_eq!(pattern_set(None), None);
    }

    #[test]
    fn english_words_break_where_the_lists_and_patterns_say() {
        // The observed list: "an=ni=ver=sary".
        assert_eq!(points("anniversary", Some("en-US"), 2, false), vec![2, 4, 7]);
        assert!(!points("hyphenation", Some("en-US"), 2, false).is_empty());
        assert!(points("hyphenation", Some("nl-BE"), 2, false).is_empty());
        assert!(!points("Silbentrennung", Some("de-DE"), 2, false).is_empty());
    }
}
