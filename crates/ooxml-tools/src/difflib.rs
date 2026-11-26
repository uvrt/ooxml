//! "Did you mean": the option a misspelt name or value most likely meant, as today's
//! Python validator chooses it (`schema.py`, `close_match`), with the parts of Python's
//! `difflib` it uses (`SequenceMatcher.ratio`, `get_close_matches`).
//!
//! Owns: the ranking of options for a "did you mean" hint.
//!
//! Must not: be used as a diff; nothing here reports opcodes. Strings are compared by
//! Unicode code point, as Python compares `str`.

use std::collections::HashMap;

/// Python's `SequenceMatcher(None, a, b).ratio()`: twice the matched characters over the
/// total.
#[must_use]
#[allow(clippy::cast_precision_loss)] // names are short
pub fn ratio(a: &[char], b: &[char]) -> f64 {
    let total = a.len() + b.len();
    if total == 0 {
        return 1.0;
    }
    let index = b_index(b);
    let matched = matched(a, b, &index, (0, a.len()), (0, b.len()));
    2.0 * matched as f64 / total as f64
}

/// Where each character of `b` is, without the characters Python's `autojunk` drops as
/// popular (more than 1% of a `b` of 200 or more).
fn b_index(b: &[char]) -> HashMap<char, Vec<usize>> {
    let mut index: HashMap<char, Vec<usize>> = HashMap::new();
    for (j, c) in b.iter().enumerate() {
        index.entry(*c).or_default().push(j);
    }
    if b.len() >= 200 {
        let most = b.len() / 100 + 1;
        index.retain(|_, places| places.len() <= most);
    }
    index
}

/// The characters `get_matching_blocks` matches between `a[alo..ahi]` and `b[blo..bhi]`.
fn matched(
    a: &[char],
    b: &[char],
    index: &HashMap<char, Vec<usize>>,
    (alo, ahi): (usize, usize),
    (blo, bhi): (usize, usize),
) -> usize {
    let (i, j, k) = longest_match(a, b, index, (alo, ahi), (blo, bhi));
    if k == 0 {
        return 0;
    }
    let mut total = k;
    if alo < i && blo < j {
        total += matched(a, b, index, (alo, i), (blo, j));
    }
    if i + k < ahi && j + k < bhi {
        total += matched(a, b, index, (i + k, ahi), (j + k, bhi));
    }
    total
}

/// Python's `find_longest_match` without junk: the longest block, earliest in `a`, then in
/// `b`, extended over the popular characters the index leaves out.
fn longest_match(
    a: &[char],
    b: &[char],
    index: &HashMap<char, Vec<usize>>,
    (alo, ahi): (usize, usize),
    (blo, bhi): (usize, usize),
) -> (usize, usize, usize) {
    let (mut best_i, mut best_j, mut best) = (alo, blo, 0usize);
    let mut lengths: HashMap<usize, usize> = HashMap::new();
    for (i, c) in a.iter().enumerate().take(ahi).skip(alo) {
        let mut next: HashMap<usize, usize> = HashMap::new();
        for &j in index.get(c).map_or(&[][..], Vec::as_slice) {
            if j < blo {
                continue;
            }
            if j >= bhi {
                break;
            }
            let k = j.checked_sub(1).and_then(|p| lengths.get(&p)).copied().unwrap_or(0) + 1;
            next.insert(j, k);
            if k > best {
                (best_i, best_j, best) = (i + 1 - k, j + 1 - k, k);
            }
        }
        lengths = next;
    }
    while best_i > alo && best_j > blo && a.get(best_i - 1) == b.get(best_j - 1) {
        best_i -= 1;
        best_j -= 1;
        best += 1;
    }
    while best_i + best < ahi && best_j + best < bhi && a.get(best_i + best) == b.get(best_j + best) {
        best += 1;
    }
    (best_i, best_j, best)
}

#[allow(clippy::cast_precision_loss)] // names are short
fn quick_ratio(a: &[char], b: &[char]) -> f64 {
    let mut counts: HashMap<char, isize> = HashMap::new();
    for c in b {
        *counts.entry(*c).or_default() += 1;
    }
    let mut matches = 0usize;
    for c in a {
        let left = counts.entry(*c).or_default();
        if *left > 0 {
            matches += 1;
        }
        *left -= 1;
    }
    2.0 * matches as f64 / (a.len() + b.len()).max(1) as f64
}

#[allow(clippy::cast_precision_loss)] // names are short
fn real_quick_ratio(a: &[char], b: &[char]) -> f64 {
    2.0 * a.len().min(b.len()) as f64 / (a.len() + b.len()).max(1) as f64
}

/// Python's `difflib.get_close_matches(word, options, 1, cutoff)`: the best option at
/// `cutoff` or above; on a tie, the greater string.
#[must_use]
pub fn close_match_difflib<'a>(word: &[char], options: &[&'a [char]], cutoff: f64) -> Option<&'a [char]> {
    let mut best: Option<(f64, &'a [char])> = None;
    for option in options {
        if real_quick_ratio(option, word) < cutoff || quick_ratio(option, word) < cutoff {
            continue;
        }
        let score = ratio(option, word);
        if score < cutoff {
            continue;
        }
        let better = match best {
            None => true,
            Some((top, text)) => score > top || (score == top && *option > text),
        };
        if better {
            best = Some((score, option));
        }
    }
    best.map(|(_, option)| option)
}

/// Python's `difflib.get_close_matches(word, options, n, cutoff)`: the best `n` options at
/// `cutoff` or above, best first; on a tie, the greater string first.
#[must_use]
pub fn close_matches(word: &str, options: &[&str], n: usize, cutoff: f64) -> Vec<String> {
    let word: Vec<char> = word.chars().collect();
    let mut scored: Vec<(f64, Vec<char>)> = Vec::new();
    for option in options {
        let option: Vec<char> = option.chars().collect();
        if real_quick_ratio(&option, &word) < cutoff || quick_ratio(&option, &word) < cutoff {
            continue;
        }
        let score = ratio(&option, &word);
        if score >= cutoff {
            scored.push((score, option));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    scored
        .into_iter()
        .take(n)
        .map(|(_, option)| option.into_iter().collect())
        .collect()
}

/// `text` lower-cased, without white space, `_` and `-`.
#[must_use]
pub fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .collect()
}

/// The option `value` most likely meant, `None` when none is close (Python's
/// `close_match`). Ranked: the same ignoring case and `-`/`_`/spaces; a prefix either way
/// (the longest shared prefix, then the nearest); an abbreviation, the letters in order
/// from the same first letter (the nearest); `get_close_matches`' nearest at 0.6. Prefixes
/// and abbreviations need three letters.
#[must_use]
pub fn close_match(value: &str, options: &[&str]) -> Option<String> {
    // Python's dict: a folded key keeps its first place and its last option.
    let mut folded: Vec<(Vec<char>, String)> = Vec::new();
    for option in options {
        let key: Vec<char> = fold(option).chars().collect();
        match folded.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = (*option).to_owned(),
            None => folded.push((key, (*option).to_owned())),
        }
    }
    let given: Vec<char> = fold(value).chars().collect();
    if let Some((_, option)) = folded.iter().find(|(k, _)| *k == given) {
        return Some(option.clone());
    }
    if given.is_empty() {
        return None;
    }
    let near = |key: &[char]| ratio(&given, key);
    let prefixed: Vec<&(Vec<char>, String)> = folded
        .iter()
        .filter(|(k, _)| k.len().min(given.len()) >= 3 && (given.starts_with(k) || k.starts_with(&given)))
        .collect();
    if let Some(found) = first_max(&prefixed, |k| (k.len().min(given.len()) as f64, near(k))) {
        return Some(found);
    }
    let abbreviated: Vec<&(Vec<char>, String)> = folded
        .iter()
        .filter(|(k, _)| {
            k.len().min(given.len()) >= 3 && k.first() == given.first() && (in_order(k, &given) || in_order(&given, k))
        })
        .collect();
    if let Some(found) = first_max(&abbreviated, |k| (0.0, near(k))) {
        return Some(found);
    }
    let keys: Vec<&[char]> = folded.iter().map(|(k, _)| k.as_slice()).collect();
    let found = close_match_difflib(&given, &keys, 0.6)?;
    folded
        .iter()
        .find(|(k, _)| k.as_slice() == found)
        .map(|(_, option)| option.clone())
}

/// The option of the first greatest score, as Python's `max`.
#[allow(clippy::cast_precision_loss)]
fn first_max(found: &[&(Vec<char>, String)], score: impl Fn(&[char]) -> (f64, f64)) -> Option<String> {
    let mut best: Option<((f64, f64), &String)> = None;
    for (key, option) in found.iter().map(|e| (&e.0, &e.1)) {
        let s = score(key);
        if best.is_none_or(|(top, _)| s > top) {
            best = Some((s, option));
        }
    }
    best.map(|(_, option)| option.clone())
}

/// Whether `short`'s letters appear in `long` in order.
fn in_order(short: &[char], long: &[char]) -> bool {
    let mut letters = long.iter();
    short.iter().all(|c| letters.any(|l| l == c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn ratio_is_pythons() {
        // difflib.SequenceMatcher(None, a, b).ratio(), from Python 3.12.
        assert!((ratio(&chars("nomral"), &chars("normal")) - 0.833_333_333_333_333_4).abs() < 1e-12);
        assert!((ratio(&chars("abcd"), &chars("bcde")) - 0.75).abs() < 1e-12);
        assert!((ratio(&chars(""), &chars("")) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn close_match_ranks_as_python() {
        let options = ["normal", "roundRect", "rect"];
        assert_eq!(close_match("Normal", &options).as_deref(), Some("normal"));
        assert_eq!(close_match("rectangle", &options).as_deref(), Some("rect"));
        assert_eq!(close_match("rndrect", &options).as_deref(), Some("roundRect"));
        assert_eq!(close_match("nomral", &options).as_deref(), Some("normal"));
        assert_eq!(close_match("xyz", &options), None);
        assert_eq!(
            close_match("spacebefore", &["space_before", "space_after"]).as_deref(),
            Some("space_before")
        );
    }
}
