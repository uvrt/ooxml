//! Owns: one copy compared with the reference it was made from (docx-agent's
//! `edit/copies.py` `compare_copy`): its blocks paired with the reference's ("Corresponding
//! blocks": by kind, then text, then style, in order, a repeat of a pattern after it), each
//! pair's differences -- `word_compare_blocks`' and the list items' and page settings' as
//! Word applies them (`super::differences`), each with its `kind` -- the blocks left
//! unpaired, and where the reference's example text is still in the copy.
//!
//! Must not: change a document, or judge whether a difference is intended.

use super::differences::{compare_settings, Compared, Run};
use super::fixes::Fixing;
use super::settings::{paragraphs_in, region, Paragraph, Reader, REPLACED};
use crate::describe::py_space;
use crate::style::compare::{compare, Block, Side};
use crate::style::compare_paragraphs::paragraph_style;
use crate::style::compare_tables::paragraph_pairs;
use ooxml_tools::ToolError;
use ooxml_word_edit::pairing::{by_text, collapse};
use ooxml_word_model::docx::text::{paragraph_text, View};
use ooxml_word_model::docx::{casefold, find_path};
use ooxml_word_model::ids::W;
use ooxml_xml::Element;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A sentence of at least this many words is example text ...
const EXAMPLE_MIN_WORDS: usize = 3;
/// ... or of at least this many characters (one long word, a code, a URL).
const EXAMPLE_MIN_CHARS: usize = 20;
/// The characters of a unit an entry shows.
const EXAMPLE_SHOWN: usize = 60;
/// The characters of a block's text a missing or extra block shows.
const DESCRIBED: usize = 40;

/// Differences that are the copy's own content, kept by design.
const OWN: [&str; 6] = [
    "rows",
    "paragraphs",
    "missing_paragraph",
    "extra_paragraph",
    "extra_block",
    "list",
];
/// Differences of another structure (`word_follow_sample`'s `structure`).
const STRUCTURE: [&str; 3] = ["kind", "columns", "merges"];

/// A block of a copy or of a reference: its id, its place in the main part and the element.
#[derive(Debug, Clone)]
pub(crate) struct CopyBlock {
    pub(crate) id: String,
    pub(crate) path: Vec<u32>,
    pub(crate) element: Arc<Element>,
}

impl CopyBlock {
    fn table(&self) -> bool {
        self.element.name().is(Some(W), "tbl")
    }

    /// Its paragraphs in document order: a paragraph block itself, a table's in its cells
    /// (in text boxes too where `boxes`).
    fn paragraphs(&self, boxes: bool) -> Vec<Paragraph> {
        if self.element.name().is(Some(W), "p") {
            return vec![(self.path.clone(), Arc::clone(&self.element))];
        }
        paragraphs_in(&self.element, &self.path, boxes)
    }
}

/// One document as a copy's comparison reads it.
pub(super) struct Document<'c, 'd> {
    pub(super) side: &'c Side<'d>,
    pub(super) reader: &'c Reader<'c, 'd>,
}

/// What lists the caller states a copy's are.
pub(super) enum RunsOn<'r> {
    /// Each numbers as its sample shows.
    No,
    /// The first copy of a run.
    First,
    /// A later copy of a run: the blocks of the copy before it.
    After(&'r [CopyBlock]),
}

/// One copy compared: `{"follows", "pairs", "differences", "example_text"}`.
///
/// # Errors
/// `internal` where a block read no longer resolves.
pub(super) fn compare_copy(
    (mine, theirs): (&Document<'_, '_>, &Document<'_, '_>),
    fixing: &Fixing<'_, '_>,
    (copy, reference): (&[CopyBlock], &[CopyBlock]),
    runs_on: &RunsOn<'_>,
) -> Result<Map<String, Value>, ToolError> {
    let Aligned {
        pairs: blocks,
        missing,
        extra,
    } = align(mine.side, copy, theirs.side, reference);
    let run = match runs_on {
        RunsOn::No => None,
        RunsOn::First => Some(Run::default()),
        RunsOn::After(before) => Some(Run::after(mine.reader, &region_of(before))),
    };
    let (settings, compared_here) =
        settings_differences((mine, theirs), fixing, (copy, reference), &blocks, run.as_ref())?;
    let mut differences: Vec<Value> = Vec::new();
    for (block, source) in &blocks {
        let found = compare(mine.side, std::slice::from_ref(&block.id), &source.id, theirs.side)
            .map_err(|e| ToolError::internal(e.message, "check:copies"))?;
        for difference in found.get("differences").and_then(Value::as_array).into_iter().flatten() {
            let Value::Object(mut difference) = difference.clone() else {
                continue;
            };
            let property = difference
                .get("property")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if REPLACED.contains(&property.as_str()) {
                if let Some(Value::Array(listed)) = difference.get("paragraphs") {
                    // Read as Word applies them here (R49): the paragraphs compared so leave it.
                    let left: Vec<Value> = listed
                        .iter()
                        .filter(|p| !p.as_str().is_some_and(|p| compared_here.contains(p)))
                        .cloned()
                        .collect();
                    if left.is_empty() {
                        continue;
                    }
                    difference.insert("paragraphs".into(), Value::Array(left));
                }
            }
            let kind = kind_of(&difference);
            difference.insert("kind".into(), Value::from(kind));
            differences.push(Value::Object(difference));
        }
        for mut difference in settings.get(&block.id).cloned().unwrap_or_default() {
            let kind = kind_of(&difference);
            difference.insert("kind".into(), Value::from(kind));
            differences.push(Value::Object(difference));
        }
    }
    for block in missing {
        let mut said = Map::new();
        said.insert("block".into(), Value::from(block.id.clone()));
        said.extend(described(theirs.side, &block.element));
        let kind = if block.table() { "structure" } else { "own" };
        differences.push(
            json!({"block": null, "property": "missing_block", "reference": said, "fix": null,
                                "kind": kind}),
        );
    }
    for block in extra {
        differences.push(json!({"block": block.id, "property": "extra_block",
                                "value": described(mine.side, &block.element), "fix": null, "kind": "own"}));
    }
    let follows = differences
        .iter()
        .all(|d| d.get("kind").and_then(Value::as_str) == Some("own"));
    let mut out = Map::new();
    out.insert("follows".into(), Value::from(follows));
    out.insert(
        "pairs".into(),
        Value::from(blocks.iter().map(|(a, b)| json!([a.id, b.id])).collect::<Vec<_>>()),
    );
    out.insert("differences".into(), Value::Array(differences));
    out.insert(
        "example_text".into(),
        Value::Array(example_text(mine.reader, copy, theirs.reader, reference)),
    );
    Ok(out)
}

/// The paragraphs of `blocks` in document order, in table cells too, not in text boxes (the
/// body's own, as its lists count).
pub(super) fn region_of(blocks: &[CopyBlock]) -> Vec<Paragraph> {
    blocks.iter().flat_map(|b| b.paragraphs(false)).collect()
}

/// The settings differences by copy block, and the ids of the paragraphs compared: each
/// pair's paragraphs of the same style compared as Word applies them.
#[allow(clippy::type_complexity)]
fn settings_differences(
    (mine, theirs): (&Document<'_, '_>, &Document<'_, '_>),
    fixing: &Fixing<'_, '_>,
    (copy, reference): (&[CopyBlock], &[CopyBlock]),
    blocks: &[(&CopyBlock, &CopyBlock)],
    run: Option<&Run>,
) -> Result<(HashMap<String, Vec<Map<String, Value>>>, HashSet<String>), ToolError> {
    let (body, reference_body) = (mine.reader.body(), theirs.reader.body());
    let mut pairs: Vec<(Paragraph, Paragraph)> = Vec::new();
    let mut ids: HashMap<Vec<u32>, (String, String)> = HashMap::new();
    let mut reference_ids: HashMap<Vec<u32>, String> = HashMap::new();
    for (block, source) in blocks {
        for (paragraph, other) in paragraphs_paired(mine.side, block, theirs.side, source)? {
            if paragraph_style(mine.side, &paragraph.1) != paragraph_style(theirs.side, &other.1) {
                continue; // the style's difference first (the module's rule)
            }
            let Some(entry) = body.ids.paragraph_at(&paragraph.0) else {
                continue;
            };
            ids.insert(paragraph.0.clone(), (block.id.clone(), entry.id));
            if let Some(found) = reference_body.ids.paragraph_at(&other.0) {
                reference_ids.insert(other.0.clone(), found.id);
            }
            pairs.push((paragraph, other));
        }
    }
    if pairs.is_empty() {
        return Ok((HashMap::new(), HashSet::new()));
    }
    let mine_settings = region(mine.reader, &region_of(copy));
    let theirs_settings = region(theirs.reader, &region_of(reference));
    let compared = Compared {
        pairs: &pairs,
        ids: &ids,
        reference_ids: &reference_ids,
        mine: &mine_settings,
        theirs: &theirs_settings,
    };
    let mut by_block: HashMap<String, Vec<Map<String, Value>>> = HashMap::new();
    for difference in compare_settings(&compared, fixing, run) {
        let block = difference.get("block").and_then(Value::as_str).unwrap_or("").to_owned();
        by_block.entry(block).or_default().push(difference);
    }
    Ok((by_block, ids.into_values().map(|(_, id)| id).collect()))
}

/// The `(paragraph, reference paragraph)` pairs `word_compare_blocks` compares for a block
/// and its reference block, each at its place in its main part.
fn paragraphs_paired(
    mine: &Side<'_>,
    block: &CopyBlock,
    theirs: &Side<'_>,
    source: &CopyBlock,
) -> Result<Vec<(Paragraph, Paragraph)>, ToolError> {
    let is_p = |b: &CopyBlock| b.element.name().is(Some(W), "p");
    if is_p(block) && is_p(source) {
        return Ok(vec![(
            (block.path.clone(), Arc::clone(&block.element)),
            (source.path.clone(), Arc::clone(&source.element)),
        )]);
    }
    if !(block.table() && source.table()) {
        return Ok(Vec::new());
    }
    let unresolved = |e: ooxml_word_model::address::AddressError| ToolError::internal(e.message, "check:copies");
    let (a, b) = (
        Block::of(mine.docx, &block.id).map_err(unresolved)?,
        Block::of(theirs.docx, &source.id).map_err(unresolved)?,
    );
    let place = |at: &[u32], below: &[u32]| [at, below].concat();
    Ok(paragraph_pairs(&a, mine, &b, theirs)
        .into_iter()
        .map(|((p, below), (o, other_below))| ((place(&a.path, &below), p), (place(&b.path, &other_below), o)))
        .collect())
}

/// A difference's `kind` (the module's "Differences").
fn kind_of(difference: &Map<String, Value>) -> &'static str {
    let name = difference.get("property").and_then(Value::as_str).unwrap_or("");
    let fixed = difference.get("fix").is_some_and(|f| !f.is_null());
    if OWN.contains(&name) {
        return "own";
    }
    match name {
        "paragraph_style" | "table_style" if fixed => "format",
        "paragraph_style" | "table_style" => "changed_by_copy",
        "widths" if fixed => "format",
        "widths" => "structure",
        _ if STRUCTURE.contains(&name) => "structure",
        _ => "format",
    }
}

/// `text` collapsed and case-folded.
fn normalised(text: &str) -> String {
    casefold(&collapse(text))
}

/// A block's kind, style name and text, normalised (a table's: its first row's).
fn signature(side: &Side<'_>, element: &Element) -> (&'static str, Option<String>, String) {
    if element.name().is(Some(W), "tbl") {
        let style =
            find_path(element, &["tblPr", "tblStyle"]).and_then(|n| side.style_name(n.attribute(Some(W), "val")));
        let row = element
            .children()
            .iter()
            .find_map(|n| n.as_element().filter(|e| e.name().is(Some(W), "tr")));
        let text = row.map_or_else(String::new, |row| {
            paragraphs_in(row, &[], true)
                .iter()
                .map(|(_, p)| paragraph_text(p, View::Current))
                .collect::<Vec<_>>()
                .join(" | ")
        });
        return ("table", style, normalised(&text));
    }
    if element.name().is(Some(W), "p") {
        return (
            "paragraph",
            paragraph_style(side, element),
            normalised(&paragraph_text(element, View::Current)),
        );
    }
    ("block", None, String::new())
}

/// A missing or extra block as reported: its kind, style and the start of its text.
fn described(side: &Side<'_>, element: &Element) -> Map<String, Value> {
    let (kind, style, text) = signature(side, element);
    let mut out = Map::new();
    out.insert("kind".into(), Value::from(kind));
    out.insert("style".into(), style.map_or(Value::Null, Value::from));
    out.insert("text".into(), Value::from(start_of(&text, DESCRIBED)));
    out
}

/// The first `count` characters of `text`, white space after them dropped, and `…` where
/// it is longer.
fn start_of(text: &str, count: usize) -> String {
    if text.chars().count() <= count {
        return text.trim_end_matches(py_space).to_owned();
    }
    let start: String = text.chars().take(count).collect();
    format!("{}\u{2026}", start.trim_end_matches(py_space))
}

/// The copy's blocks paired with the reference's (the module's "Corresponding blocks").
struct Aligned<'b> {
    /// `(copy block, reference block)` pairs, in the copy's order.
    pairs: Vec<(&'b CopyBlock, &'b CopyBlock)>,
    /// The reference's blocks left unpaired.
    missing: Vec<&'b CopyBlock>,
    /// The copy's.
    extra: Vec<&'b CopyBlock>,
}

/// A table of `rows` x `columns` numbers, read as 0 outside it.
struct Table {
    columns: usize,
    cells: Vec<usize>,
}

impl Table {
    fn new(rows: usize, columns: usize) -> Self {
        Table {
            columns,
            cells: vec![0; rows * columns],
        }
    }

    fn at(&self, i: usize, j: usize) -> usize {
        self.cells.get(i * self.columns + j).copied().unwrap_or(0)
    }

    fn set(&mut self, i: usize, j: usize, value: usize) {
        if let Some(cell) = self.cells.get_mut(i * self.columns + j) {
            *cell = value;
        }
    }
}

/// The copy's blocks paired with the reference's: blocks pair only with blocks of their
/// kind, in order, as many matching by text as can, then as many more by style; a copy
/// block after a paired one, of its kind and style, repeats it.
fn align<'b>(
    mine_side: &Side<'_>,
    copy: &'b [CopyBlock],
    theirs_side: &Side<'_>,
    reference: &'b [CopyBlock],
) -> Aligned<'b> {
    let (m, n) = (copy.len(), reference.len());
    let mine: Vec<_> = copy.iter().map(|b| signature(mine_side, &b.element)).collect();
    let theirs: Vec<_> = reference.iter().map(|b| signature(theirs_side, &b.element)).collect();
    let weight = m + n + 1;
    let mut scores = Table::new(m, n);
    for (i, a) in mine.iter().enumerate() {
        for (j, b) in theirs.iter().enumerate() {
            if a.0 == b.0 {
                scores.set(
                    i,
                    j,
                    weight * usize::from(by_text(&a.2, &b.2)) + usize::from(a.1 == b.1),
                );
            }
        }
    }
    let mut best = Table::new(m + 1, n + 1);
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            let score = scores.at(i, j);
            let taken = if score > 0 { score + best.at(i + 1, j + 1) } else { 0 };
            best.set(i, j, best.at(i + 1, j).max(best.at(i, j + 1)).max(taken));
        }
    }
    let mut aligned: Vec<(usize, usize)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < m && j < n {
        let score = scores.at(i, j);
        if score > 0 && best.at(i, j) == score + best.at(i + 1, j + 1) {
            aligned.push((i, j));
            (i, j) = (i + 1, j + 1);
        } else if best.at(i, j) == best.at(i, j + 1) {
            j += 1;
        } else {
            i += 1;
        }
    }
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut previous: Option<usize> = None;
    for (i, own) in mine.iter().enumerate() {
        if let Some((_, j)) = aligned.iter().find(|(a, _)| *a == i) {
            previous = Some(*j);
            pairs.push((i, *j));
        } else if let Some(j) = previous.filter(|j| theirs.get(*j).is_some_and(|t| own.0 == t.0 && own.1 == t.1)) {
            pairs.push((i, j)); // a repeat of the pattern
        }
    }
    Aligned {
        pairs: pairs
            .iter()
            .filter_map(|(i, j)| copy.get(*i).zip(reference.get(*j)))
            .collect(),
        missing: reference
            .iter()
            .enumerate()
            .filter(|(j, _)| !aligned.iter().any(|(_, b)| b == j))
            .map(|(_, b)| b)
            .collect(),
        extra: copy
            .iter()
            .enumerate()
            .filter(|(i, _)| !pairs.iter().any(|(a, _)| a == i))
            .map(|(_, b)| b)
            .collect(),
    }
}

/// A reference's example unit: the unit, the paragraph it is in, whether from a table's
/// unmarked first row.
type Unit = (String, String, bool);

/// The reference's example units (the module's rule), each once, in order.
fn example_units(reader: &Reader<'_, '_>, blocks: &[CopyBlock]) -> Vec<Unit> {
    let ids = &reader.body().ids;
    let root = reader.body().root();
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for block in blocks {
        for (path, paragraph) in block.paragraphs(true) {
            let mut first_row = false;
            if let Some((row, table, at)) = nearest_row(root, &path) {
                let header = find_path(&row, &["trPr", "tblHeader"]).is_some_and(|h| {
                    !matches!(
                        h.attribute(Some(W), "val").unwrap_or("1").to_lowercase().as_str(),
                        "0" | "false" | "off"
                    )
                });
                if header {
                    continue;
                }
                first_row = table
                    .children()
                    .iter()
                    .position(|n| n.as_element().is_some_and(|e| e.name().is(Some(W), "tr")))
                    == Some(at);
            }
            let text = paragraph_text(&paragraph, View::Current);
            for sentence in sentences(&text) {
                let unit = normalised(sentence);
                if unit.is_empty() || seen.contains(&unit) {
                    continue;
                }
                if unit.split(' ').count() < EXAMPLE_MIN_WORDS && unit.chars().count() < EXAMPLE_MIN_CHARS {
                    continue;
                }
                seen.insert(unit.clone());
                let id = ids.paragraph_at(&path).map(|p| p.id).unwrap_or_default();
                out.push((unit, id, first_row));
            }
        }
    }
    out
}

/// The row nearest above the node at `path` of the tree at `root`, its parent and its place
/// there (by place, never by pointer: identical subtrees may be one element).
fn nearest_row(root: &Arc<Element>, path: &[u32]) -> Option<(Arc<Element>, Arc<Element>, usize)> {
    let mut chain: Vec<Arc<Element>> = vec![Arc::clone(root)];
    let mut node = Arc::clone(root);
    for k in path {
        node = Arc::clone(node.children().get(*k as usize)?.as_element()?);
        chain.push(Arc::clone(&node));
    }
    chain.pop(); // the paragraph itself
    let at = chain.iter().rposition(|e| e.name().is(Some(W), "tr"))?;
    let parent = at.checked_sub(1).and_then(|p| chain.get(p))?;
    let place = *path.get(at.checked_sub(1)?)? as usize;
    Some((Arc::clone(chain.get(at)?), Arc::clone(parent), place))
}

/// `text` split into sentences as docx-agent splits it: after `.`, `!`, `?` or `…` and
/// white space, and at a line break, a tab or a page break.
fn sentences(text: &str) -> Vec<&str> {
    let breaks = |c: char| matches!(c, '\n' | '\t' | '\u{b}' | '\u{c}' | '\r');
    let mut out = Vec::new();
    let mut start = 0;
    let mut previous: Option<char> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let after_stop = previous.is_some_and(|p| matches!(p, '.' | '!' | '?' | '\u{2026}'));
        let run: Option<fn(char) -> bool> = if after_stop && py_space(c) {
            Some(py_space)
        } else if breaks(c) {
            Some(|c| matches!(c, '\n' | '\t' | '\u{b}' | '\u{c}' | '\r'))
        } else {
            None
        };
        previous = Some(c);
        let Some(run) = run else { continue };
        out.push(text.get(start..at).unwrap_or(""));
        while chars.next_if(|(_, c)| run(*c)).is_some() {}
        start = chars.peek().map_or(text.len(), |(i, _)| *i);
        previous = None;
    }
    out.push(text.get(start..).unwrap_or(""));
    out
}

/// Whether `unit` is in `text` as whole words (not inside a longer word).
fn holds(text: &str, unit: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    text.contains(unit)
        && text.char_indices().any(|(at, _)| {
            let Some(rest) = text.get(at..) else { return false };
            if !rest.starts_with(unit) {
                return false;
            }
            let before = text.get(..at).and_then(|t| t.chars().next_back());
            let after = rest.get(unit.len()..).and_then(|t| t.chars().next());
            !before.is_some_and(word) && !after.is_some_and(word)
        })
}

/// Every copy paragraph holding example units of the reference: `{"para", "text",
/// "reference"}` (`first_row` where one is from a table's unmarked first row), in the
/// copy's order.
fn example_text(
    reader: &Reader<'_, '_>,
    copy: &[CopyBlock],
    theirs: &Reader<'_, '_>,
    reference: &[CopyBlock],
) -> Vec<Value> {
    let units = example_units(theirs, reference);
    if units.is_empty() {
        return Vec::new();
    }
    let ids = &reader.body().ids;
    let mut out = Vec::new();
    for block in copy {
        for (path, paragraph) in block.paragraphs(true) {
            let text = normalised(&paragraph_text(&paragraph, View::Current));
            if text.is_empty() {
                continue;
            }
            let mut found: Option<Map<String, Value>> = None;
            for (unit, source, first) in &units {
                if unit.chars().count() > text.chars().count() || !holds(&text, unit) {
                    continue;
                }
                let entry = found.get_or_insert_with(|| {
                    let mut entry = Map::new();
                    let id = ids.paragraph_at(&path).map(|p| p.id);
                    entry.insert("para".into(), id.map_or(Value::Null, Value::from));
                    entry.insert("text".into(), Value::Array(Vec::new()));
                    entry.insert("reference".into(), Value::from(source.as_str()));
                    entry
                });
                if let Some(Value::Array(texts)) = entry.get_mut("text") {
                    texts.push(Value::from(start_of(unit, EXAMPLE_SHOWN)));
                }
                if *first {
                    entry.insert("first_row".into(), Value::from(true));
                }
            }
            if let Some(entry) = found {
                out.push(Value::Object(entry));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{holds, sentences, start_of};

    #[test]
    fn sentences_split_after_a_stop_and_at_breaks() {
        assert_eq!(
            sentences("One two. Three\tfour!  Five"),
            vec!["One two.", "Three", "four!", "Five"]
        );
        assert_eq!(sentences("a.b c"), vec!["a.b c"]);
        assert_eq!(sentences("end. "), vec!["end.", ""]);
    }

    #[test]
    fn units_are_held_as_whole_words() {
        assert!(holds("write the part here", "the part"));
        assert!(!holds("write thee parts", "the part"));
        assert_eq!(start_of("short", 40), "short");
        assert_eq!(start_of("abcdef ", 3), "abc\u{2026}");
    }
}
