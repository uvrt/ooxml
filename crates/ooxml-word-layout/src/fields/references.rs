//! Owns: the body's `SEQ` and `REF` fields, computed once before the page passes: they do
//! not depend on the pages (docx2svg's `fields.sequence_values`, `sequence_switches`,
//! `sequence_step`, `reference_runs`, `cross_references`): a `SEQ` counted as Word counts
//! it, a `REF` replaced by its bookmark's text in its own runs' formats -- with
//! `\* CHARFORMAT` in the field's run's, with `\* MERGEFORMAT` word by word in the cached
//! result's -- and its case switches applied. A field that cannot be computed keeps
//! Word's cached result.
//!
//! Must not: compute a page-number field (`crate::fields` does, against the pages) or
//! `STYLEREF` (not ported: see lane L3a's hand-back).

use super::format::{argument, number_text, switches, Uncomputable};
use crate::flow::body_paragraphs;
use ooxml_word_model::model::{Paragraph, Run, RunProps};
use ooxml_word_model::WordDocument;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

fn uncomputable(why: impl Into<String>) -> Uncomputable {
    Uncomputable(why.into())
}

/// The tokens of a `SEQ` instruction: quoted strings, switches, words.
fn tokens(instruction: &str) -> Vec<String> {
    let chars: Vec<char> = instruction.chars().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while let Some(&c) = chars.get(k) {
        if c.is_whitespace() {
            k += 1;
        } else if c == '"' {
            let end = chars.iter().skip(k + 1).position(|d| *d == '"').map(|p| k + 1 + p);
            match end {
                Some(end) => {
                    out.push(chars.get(k..=end).unwrap_or_default().iter().collect());
                    k = end + 1;
                }
                None => k += 1,
            }
        } else if c == '\\' {
            match chars.get(k + 1) {
                Some(d) if !d.is_whitespace() => {
                    out.push(format!("\\{d}"));
                    k += 2;
                }
                _ => k += 1,
            }
        } else {
            let end = chars
                .iter()
                .skip(k)
                .position(|d| d.is_whitespace() || *d == '"' || *d == '\\')
                .map_or(chars.len(), |p| k + p);
            out.push(chars.get(k..end).unwrap_or_default().iter().collect());
            k = end;
        }
    }
    out
}

/// A switch and its argument.
type Switch = (char, Option<String>);

/// A `SEQ` instruction's identifier and its switches in order (`sequence_switches`).
fn sequence_switches(instruction: &str) -> (Option<String>, Vec<Switch>) {
    let tokens = tokens(instruction);
    let mut name = None;
    let mut found = Vec::new();
    let mut k = usize::from(tokens.first().is_some_and(|t| t.eq_ignore_ascii_case("SEQ")));
    while let Some(token) = tokens.get(k) {
        k += 1;
        if let Some(rest) = token.strip_prefix('\\') {
            let c = rest.chars().next().unwrap_or(' ');
            let key = if "#*@!".contains(c) { c } else { c.to_ascii_lowercase() };
            let mut value = None;
            if "#*@rs".contains(key) {
                if let Some(next) = tokens.get(k) {
                    value = Some(next.trim_matches('"').to_owned());
                    k += 1;
                }
            }
            found.push((key, value));
        } else if name.is_none() {
            name = Some(token.trim_matches('"').to_owned());
        }
    }
    (name, found)
}

/// One `SEQ` counted (`sequence_step`).
fn sequence_step(
    instruction: &str,
    name: &str,
    found: &[Switch],
    counts: &mut HashMap<String, i64>,
    restarted: &HashSet<String>,
) -> Result<String, Uncomputable> {
    if name.is_empty() {
        return Err(uncomputable("a SEQ field names no sequence"));
    }
    if restarted.contains(name) {
        return Err(uncomputable(format!(
            "the sequence {name:?} restarts at heading levels (\\s)"
        )));
    }
    let flag = |key: char| found.iter().find(|(k, _)| *k == key);
    if let Some((_, restart)) = flag('r') {
        let value = restart
            .as_deref()
            .filter(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
            .and_then(|r| r.parse().ok())
            .ok_or_else(|| uncomputable("\\r takes a number"))?;
        counts.insert(name.to_owned(), value);
    } else if flag('c').is_none() {
        *counts.entry(name.to_owned()).or_insert(0) += 1;
    }
    let value = counts.get(name).copied().unwrap_or(0);
    if flag('h').is_some() {
        return Ok(String::new());
    }
    let keys: Vec<char> = found.iter().map(|(k, _)| *k).collect();
    if let Some(star) = keys.iter().position(|k| *k == '*') {
        if keys.iter().skip(star).any(|k| *k == '#') {
            return Err(uncomputable("a \\# after a \\* is Word's error text"));
        }
    }
    number_text(instruction, value, None)
}

/// Every `SEQ` field's text, counted in document order (`sequence_values`).
fn sequence_values(paragraphs: &[Arc<Paragraph>]) -> BTreeMap<(usize, usize), Result<String, Uncomputable>> {
    let mut parsed = Vec::new();
    for (block, paragraph) in paragraphs.iter().enumerate() {
        for (k, run) in paragraph.runs.iter().enumerate() {
            if let Some(field) = run.field.as_ref().filter(|f| &*f.keyword == "SEQ") {
                parsed.push((
                    block,
                    k,
                    field.instruction.clone(),
                    sequence_switches(&field.instruction),
                ));
            }
        }
    }
    let restarted: HashSet<String> = parsed
        .iter()
        .filter(|(_, _, _, (_, found))| found.iter().any(|(k, _)| *k == 's'))
        .map(|(_, _, _, (name, _))| name.as_deref().unwrap_or("").to_uppercase())
        .collect();
    let mut counts = HashMap::new();
    parsed
        .into_iter()
        .map(|(block, k, instruction, (name, found))| {
            let name = name.unwrap_or_default().to_uppercase();
            (
                (block, k),
                sequence_step(&instruction, &name, &found, &mut counts, &restarted),
            )
        })
        .collect()
}

/// Bookmark name -> the `(paragraph, run)` of its start and of its end, the first of a
/// name winning (`bookmarks`).
type Marks = HashMap<String, ((usize, usize), Option<(usize, usize)>)>;

fn marks(paragraphs: &[Arc<Paragraph>]) -> Marks {
    let mut starts: HashMap<String, (Option<String>, (usize, usize))> = HashMap::new();
    let mut ends: HashMap<Option<String>, (usize, usize)> = HashMap::new();
    for (block, paragraph) in paragraphs.iter().enumerate() {
        for mark in paragraph.bookmarks.iter() {
            let id = mark.id.as_deref().map(str::to_owned);
            match (mark.start, mark.name.as_deref().filter(|n| !n.is_empty())) {
                (true, Some(name)) => {
                    starts.entry(name.to_owned()).or_insert((id, (block, mark.run)));
                }
                (false, _) => {
                    ends.entry(id).or_insert((block, mark.run));
                }
                (true, None) => {}
            }
        }
    }
    starts
        .into_iter()
        .map(|(name, (id, start))| (name, (start, ends.get(&id).copied())))
        .collect()
}

/// The instruction's letter switches (`_flags`), on its unquoted text.
fn letters(instruction: &str) -> Vec<char> {
    let mut quoted = false;
    let mut out = Vec::new();
    let mut previous = ' ';
    for c in instruction.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted && previous == '\\' && c.is_ascii_alphabetic() {
            out.push(c.to_ascii_lowercase());
        }
        previous = if quoted { ' ' } else { c };
    }
    out
}

/// `text` with the instruction's case switches (`\* Upper`, `Lower`, `FirstCap`, `Caps`)
/// applied (`apply_case`).
pub(crate) fn apply_case(text: &str, instruction: &str) -> String {
    let mut text = text.to_owned();
    for (kind, argument) in switches(instruction) {
        if kind != '*' {
            continue;
        }
        let first_cap = |w: &str| {
            let mut c = w.chars();
            c.next()
                .map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
        };
        text = match argument.to_lowercase().as_str() {
            "upper" => text.to_uppercase(),
            "lower" => text.to_lowercase(),
            "firstcap" => first_cap(&text),
            "caps" => text.split(' ').map(first_cap).collect::<Vec<_>>().join(" "),
            _ => text,
        };
    }
    text
}

/// Where the words of a text start and end (`\S+`), in characters.
fn words(text: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (k, c) in text.iter().enumerate() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(k),
            (true, Some(s)) => {
                out.push((s, k));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    out
}

type Props = Option<Arc<RunProps>>;

fn same_props(a: &Props, b: &Props) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => Arc::ptr_eq(x, y),
        (None, None) => true,
        _ => false,
    }
}

/// A `REF` field's bookmark's runs and its `\*` words, or why it cannot be computed.
fn reference_source<'p>(
    paragraphs: &'p [Arc<Paragraph>],
    instruction: &str,
    marks: &Marks,
) -> Result<(&'p [Run], Vec<String>), Uncomputable> {
    let name = argument(instruction).filter(|n| marks.contains_key(n));
    let (start, end) = name
        .and_then(|n| marks.get(&n).copied())
        .ok_or_else(|| uncomputable("its bookmark is not in the body"))?;
    if letters(instruction).iter().any(|l| *l != 'h') {
        return Err(uncomputable("a switch that is not computed"));
    }
    let (end_block, end_run) = end.ok_or_else(|| uncomputable("its bookmark has no end"))?;
    if end_block != start.0 {
        return Err(uncomputable("its bookmark spans paragraphs"));
    }
    let source = paragraphs
        .get(start.0)
        .and_then(|p| p.runs.get(start.1..end_run.max(start.1)))
        .unwrap_or_default();
    for piece in source {
        if !piece.breaks.is_empty() || !piece.drawings.is_empty() || !piece.anchors.is_empty() {
            return Err(uncomputable("its bookmark holds a tab, a break or an object"));
        }
        if piece
            .field
            .as_ref()
            .is_some_and(|f| super::PAGE_FIELDS.contains(&&*f.keyword))
        {
            return Err(uncomputable("its bookmark holds a page-number field"));
        }
    }
    let words_of: Vec<String> = switches(instruction)
        .into_iter()
        .filter(|(k, _)| *k == '*')
        .map(|(_, a)| a.to_lowercase())
        .collect();
    let known = ["mergeformat", "charformat", "upper", "lower", "firstcap", "caps"];
    if let Some(w) = words_of.iter().find(|w| !known.contains(&w.as_str())) {
        return Err(uncomputable(format!("the switch \\* {w} is not measured on REF")));
    }
    Ok((source, words_of))
}

/// Each character's run properties: the source runs', all the field's run's under
/// `CHARFORMAT`, the cached result's word by word under `MERGEFORMAT`.
fn formats(run: &Run, source: &[Run], words_of: &[String], text: &[char]) -> Vec<Props> {
    let has = |w: &str| words_of.iter().any(|x| x == w);
    if has("charformat") {
        return vec![run.properties.clone(); text.len()];
    }
    let mut chars: Vec<Props> = source
        .iter()
        .flat_map(|p| std::iter::repeat_n(p.properties.clone(), p.text.chars().count()))
        .collect();
    let cached = run.field.as_ref().map(|f| Arc::clone(&f.cached));
    if let (true, Some(cached)) = (has("mergeformat"), cached.filter(|c| !c.is_empty())) {
        let old: Vec<Props> = cached
            .iter()
            .flat_map(|(t, p)| std::iter::repeat_n(p.clone(), t.chars().count()))
            .collect();
        let old_text: Vec<char> = cached.iter().flat_map(|(t, _)| t.chars().collect::<Vec<_>>()).collect();
        let old_words: Vec<Props> = words(&old_text)
            .iter()
            .filter_map(|(s, _)| old.get(*s).cloned())
            .collect();
        for ((s, e), props) in words(text).into_iter().zip(old_words) {
            for slot in chars.iter_mut().take(e).skip(s) {
                *slot = props.clone();
            }
        }
    }
    chars
}

/// What Word draws for a `REF` field's run: its bookmark's text as runs, one per stretch
/// of characters of the same properties (`reference_runs`).
fn reference_runs(paragraphs: &[Arc<Paragraph>], run: &Run, marks: &Marks) -> Result<Vec<Run>, Uncomputable> {
    let field = run
        .field
        .as_ref()
        .ok_or_else(|| uncomputable("the run is not a field's"))?;
    let (source, words_of) = reference_source(paragraphs, &field.instruction, marks)?;
    let joined: String = source.iter().map(|p| &*p.text).collect();
    let text: Vec<char> = apply_case(&joined, &field.instruction).chars().collect();
    let chars = formats(run, source, &words_of, &text);
    if text.is_empty() {
        return Ok(vec![Run {
            text: Arc::from(""),
            field: None,
            ..run.clone()
        }]);
    }
    let mut out = Vec::new();
    let mut begin = 0;
    for index in 1..=text.len() {
        let split = match (chars.get(index), chars.get(begin)) {
            (Some(a), Some(b)) => !same_props(a, b),
            _ => true,
        };
        if split {
            out.push(Run {
                text: Arc::from(
                    text.get(begin..index)
                        .unwrap_or_default()
                        .iter()
                        .collect::<String>()
                        .as_str(),
                ),
                properties: chars.get(begin).cloned().flatten(),
                path: run.path.clone(),
                toc_link: run.toc_link,
                ..Run::default()
            });
            begin = index;
        }
    }
    Ok(out)
}

/// Paragraphs with their `SEQ` and `REF` fields computed, kept between versions: the same
/// paragraph computed alike is the same `Arc`, so its measure is kept by its `Arc`.
#[derive(Debug, Default)]
pub struct Computed {
    kept: HashMap<usize, (Arc<Paragraph>, Arc<Paragraph>)>,
}

impl Computed {
    fn keep(&mut self, original: &Arc<Paragraph>, result: Paragraph, used: &mut HashSet<usize>) -> Arc<Paragraph> {
        let key = Arc::as_ptr(original) as usize;
        used.insert(key);
        if let Some((was, kept)) = self.kept.get(&key) {
            if Arc::ptr_eq(was, original) && **kept == result {
                return Arc::clone(kept);
            }
        }
        let result = Arc::new(result);
        self.kept.insert(key, (Arc::clone(original), Arc::clone(&result)));
        result
    }
}

fn has_cross_references(paragraphs: &[Arc<Paragraph>]) -> bool {
    paragraphs.iter().any(|p| {
        p.runs
            .iter()
            .any(|r| r.field.as_ref().is_some_and(|f| matches!(&*f.keyword, "SEQ" | "REF")))
    })
}

/// A paragraph's `REF` fields replaced by their runs, its bookmarks moved with them.
fn expanded(paragraph: &Paragraph, counted: &[Arc<Paragraph>], marks: &Marks) -> Option<Paragraph> {
    if !paragraph
        .runs
        .iter()
        .any(|r| r.field.as_ref().is_some_and(|f| &*f.keyword == "REF"))
    {
        return None;
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut index_of = Vec::with_capacity(paragraph.runs.len() + 1);
    for run in paragraph.runs.iter() {
        index_of.push(runs.len());
        let is_ref = run.field.as_ref().is_some_and(|f| &*f.keyword == "REF");
        match is_ref.then(|| reference_runs(counted, run, marks)) {
            Some(Ok(replaced)) => runs.extend(replaced),
            _ => runs.push(run.clone()),
        }
    }
    index_of.push(runs.len());
    let bookmarks: Vec<_> = paragraph
        .bookmarks
        .iter()
        .map(|b| {
            let mut b = b.clone();
            b.run = index_of.get(b.run).copied().unwrap_or(b.run);
            b
        })
        .collect();
    Some(Paragraph {
        runs: runs.into(),
        bookmarks: bookmarks.into(),
        ..paragraph.clone()
    })
}

/// The document with its body's `SEQ` fields counted and its `REF` fields replaced by
/// the runs Word draws for them (`cross_references`); a field that cannot be computed is
/// left as Word cached it. Paragraphs computed alike before are taken from `kept`.
pub fn cross_references(document: &WordDocument, kept: &mut Computed) -> WordDocument {
    let (paragraphs, _) = body_paragraphs(&document.body);
    if !has_cross_references(&paragraphs) {
        kept.kept.clear();
        return document.clone();
    }
    let sequences = sequence_values(&paragraphs);
    let mut counted: Vec<Arc<Paragraph>> = paragraphs.clone();
    let mut by_block: BTreeMap<usize, Vec<(usize, String)>> = BTreeMap::new();
    for ((block, k), value) in sequences {
        if let Ok(text) = value {
            by_block.entry(block).or_default().push((k, text));
        }
    }
    for (block, values) in &by_block {
        if let Some(p) = counted.get_mut(*block) {
            let runs: Vec<Run> = p
                .runs
                .iter()
                .enumerate()
                .map(|(k, run)| match values.iter().find(|(r, _)| *r == k) {
                    Some((_, text)) => Run {
                        text: Arc::from(text.as_str()),
                        ..run.clone()
                    },
                    None => run.clone(),
                })
                .collect();
            *p = Arc::new(Paragraph {
                runs: runs.into(),
                ..(**p).clone()
            });
        }
    }
    let marks = marks(&counted);
    let finals: Vec<Option<Paragraph>> = counted
        .iter()
        .enumerate()
        .map(|(block, p)| match expanded(p, &counted, &marks) {
            Some(e) => Some(e),
            None if by_block.contains_key(&block) => Some((**p).clone()),
            None => None,
        })
        .collect();
    let mut used = HashSet::new();
    let mut change = |index: usize, original: &Arc<Paragraph>| -> Option<Arc<Paragraph>> {
        let result = finals.get(index).cloned().flatten()?;
        Some(kept.keep(original, result, &mut used))
    };
    let mut walk = super::Rebuild {
        next: 0,
        change: &mut change,
    };
    let body: Vec<_> = document.body.iter().map(|b| walk.block(b)).collect();
    kept.kept.retain(|k, _| used.contains(k));
    let mut out = document.clone();
    out.body = body.into();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_switches_read_as_word_reads_them() {
        let (name, found) = sequence_switches(r#"SEQ \# "00" Item"#);
        assert_eq!(name.as_deref(), Some("Item"));
        assert_eq!(found, vec![('#', Some("00".to_owned()))]);
        let (name, found) = sequence_switches(r"SEQ \r 7 Item \h");
        assert_eq!(name.as_deref(), Some("Item"));
        assert_eq!(found, vec![('r', Some("7".to_owned())), ('h', None)]);
    }

    #[test]
    fn sequences_count() {
        let mut counts = HashMap::new();
        let none = HashSet::new();
        let step = |i: &str, c: &mut HashMap<String, i64>| {
            let (name, found) = sequence_switches(i);
            sequence_step(i, &name.unwrap_or_default().to_uppercase(), &found, c, &none)
        };
        assert_eq!(step("SEQ Figure", &mut counts).ok().as_deref(), Some("1"));
        assert_eq!(step("SEQ Figure", &mut counts).ok().as_deref(), Some("2"));
        assert_eq!(step(r"SEQ Figure \c", &mut counts).ok().as_deref(), Some("2"));
        assert_eq!(step(r"SEQ Figure \r 5", &mut counts).ok().as_deref(), Some("5"));
        assert_eq!(step(r"SEQ Figure \h", &mut counts).ok().as_deref(), Some(""));
        assert!(step("SEQ", &mut counts).is_err());
    }
}
