//! Owns: where each value written by reference into `word_insert_markdown`'s Markdown went
//! (docx-agent's `field_text`: `with_markdown`, `placed_blocks`, `_alone`): a value that has
//! Markdown of its own (the host read it from its source: [`Written::markdown`]) written as
//! it -- standing alone between blank lines as its Markdown, inside a sentence as its text
//! on one line, escaped -- and the blocks each value standing alone made, found by reading
//! each share of the Markdown alone ([`markdown_shape`]) and lining that up with the blocks
//! the call made; where they do not line up, none (the reply then says so).
//!
//! Must not: read a value's source (the host's), or keep anything (the tool keeps what this
//! finds through [`crate::written`]).

use crate::written::{py_space, Written};
use ooxml_word_edit::{markdown_shape, Made};
use serde_json::Value;
use std::collections::BTreeMap;

/// `markdown` (the text as the call gave it, each reference written as its value's text)
/// with each value of `facts` that has Markdown of its own written as it; the values, their
/// places and texts in the new Markdown (in the order of their places where any was
/// rewritten, else as given).
#[must_use]
pub(super) fn with_markdown(markdown: &str, facts: &[&Written]) -> (String, Vec<Written>) {
    let chars: Vec<char> = markdown.chars().collect();
    let mut sorted: Vec<&Written> = facts.to_vec();
    sorted.sort_by_key(|f| f.start);
    let mut out = String::new();
    let mut placed = Vec::new();
    let (mut at, mut shift, mut rewritten) = (0usize, 0isize, false);
    for fact in sorted {
        let start = fact.start.saturating_add_signed(shift);
        let Some(own) = &fact.markdown else {
            placed.push(moved(fact, start, &fact.text));
            continue;
        };
        let found = if alone(&chars, fact.start, fact.end) {
            own.clone()
        } else {
            let line = chars.get(..fact.start).unwrap_or_default();
            let line = line.rsplit(|c| *c == '\n').next().unwrap_or_default();
            let line_start = line.iter().all(|c| py_space(*c));
            plain_inline(&one_line(&fact.text), line_start).replace('|', "\\|")
        };
        out.extend(chars.get(at..fact.start).unwrap_or_default());
        out.push_str(&found);
        at = fact.end;
        placed.push(moved(fact, start, &found));
        let length = found.chars().count();
        shift += isize::try_from(length).unwrap_or(0) - isize::try_from(fact.end - fact.start).unwrap_or(0);
        rewritten = true;
    }
    if !rewritten {
        return (markdown.to_owned(), facts.iter().map(|f| (*f).clone()).collect());
    }
    out.extend(chars.get(at..).unwrap_or_default());
    (out, placed)
}

/// `fact` at `start` in the new Markdown, written as `text` (what a reply says of it too).
fn moved(fact: &Written, start: usize, text: &str) -> Written {
    let mut out = fact.clone();
    out.start = start;
    out.end = start + text.chars().count();
    text.clone_into(&mut out.text);
    if out.brief.contains_key("text") {
        out.brief.insert("text".into(), Value::from(text));
    }
    out
}

/// `text`'s lines, stripped, the empty ones left out, joined by a space (Python's
/// `" ".join(l.strip() for l in text.splitlines() if l.strip())`).
fn one_line(text: &str) -> String {
    let breaks = |c: char| {
        matches!(
            c,
            '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}'..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
        )
    };
    text.split(breaks)
        .map(|line| line.trim_matches(py_space))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Characters a backslash escapes in plain text (ooxml-edit's `markdown.ESCAPED`).
const ESCAPED: &str = "\\`*_[]<~";

/// Plain `text` as one line of Markdown that reads back as it (ooxml-edit's
/// `inline_markdown` of one unmarked run): Markdown's characters escaped, an `&` that starts
/// an entity too; `line_start`: a marker at its start escaped.
fn plain_inline(text: &str, line_start: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for (k, c) in text.char_indices() {
        if ESCAPED.contains(c) {
            out.push('\\');
            out.push(c);
        } else if c == '&' && entity(text.get(k..).unwrap_or_default()) {
            out.push_str("\\&");
        } else {
            out.push(c);
        }
    }
    if line_start {
        escape_line_start(&out)
    } else {
        out
    }
}

/// Whether `text` starts with a character reference (`&#38;`, `&#x26;`, `&amp;`).
fn entity(text: &str) -> bool {
    let Some(body) = text.strip_prefix('&') else {
        return false;
    };
    let Some(end) = body.find(';') else { return false };
    let name = body.get(..end).unwrap_or_default();
    let all = |s: &str, f: fn(&u8) -> bool| s.as_bytes().iter().all(f);
    let fits = |s: &str, low: usize, high: usize| (low..=high).contains(&s.len());
    if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        if fits(hex, 1, 6) && all(hex, u8::is_ascii_hexdigit) {
            return true;
        }
    }
    if let Some(number) = name.strip_prefix('#') {
        return fits(number, 1, 7) && all(number, u8::is_ascii_digit);
    }
    name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && fits(name, 2, 32)
        && all(name, u8::is_ascii_alphanumeric)
}

/// A line's Markdown with a marker at its start (an item's, a heading's, a quote's)
/// escaped, so that it is text (ooxml-edit's `_escape_line_start`).
fn escape_line_start(text: &str) -> String {
    let ends_marker = |rest: &str| rest.is_empty() || rest.starts_with([' ', '\t']);
    let bytes = text.as_bytes();
    let first = bytes.first().copied();
    if matches!(first, Some(b'+' | b'*' | b'-')) && ends_marker(text.get(1..).unwrap_or_default())
        || first == Some(b'>')
    {
        return format!("\\{text}");
    }
    let hashes = bytes.iter().take_while(|b| **b == b'#').count();
    if (1..=6).contains(&hashes) && ends_marker(text.get(hashes..).unwrap_or_default()) {
        return format!("\\{text}");
    }
    let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if (1..=9).contains(&digits) {
        let (number, rest) = text.split_at(digits);
        if let Some(after) = rest.strip_prefix(['.', ')']) {
            if ends_marker(after) {
                let mark = rest.get(..1).unwrap_or_default();
                return format!("{number}\\{mark}{after}");
            }
        }
    }
    text.to_owned()
}

/// Whether `text[start..end]` stands alone: blank lines (or the text's ends) on both sides,
/// spaces aside.
fn alone(text: &[char], start: usize, end: usize) -> bool {
    let tab = |c: &char| *c == ' ' || *c == '\t';
    let before = text.get(..start).unwrap_or_default();
    let kept = before.len() - before.iter().rev().take_while(|c| tab(c)).count();
    let before = before.get(..kept).unwrap_or_default();
    let after = text.get(end..).unwrap_or_default();
    let after = after
        .get(after.iter().take_while(|c| tab(c)).count()..)
        .unwrap_or_default();
    let blank = |part: &[char]| part.iter().all(|c| py_space(*c));
    // a line break, spaces and tabs, a line break
    fn broken<'a>(mut part: impl Iterator<Item = &'a char>) -> bool {
        part.next() == Some(&'\n') && part.find(|c| **c != ' ' && **c != '\t') == Some(&'\n')
    }
    (blank(before) || broken(before.iter().rev())) && (blank(after) || broken(after.iter()))
}

/// `{index in facts: the blocks it made}` for the values standing alone in `markdown`
/// (each value's place in it), and whether the blocks made (`blocks`) lined up. A value's
/// blocks are its share of `blocks` (a leading separator, after a table already there, and
/// a trailing paragraph, after a table at a container's end, aside).
#[must_use]
pub(super) fn placed_blocks(
    markdown: &str,
    facts: &[Written],
    blocks: &[String],
) -> (BTreeMap<usize, Vec<String>>, bool) {
    let chars: Vec<char> = markdown.chars().collect();
    let mut standing: Vec<(usize, &Written)> = facts
        .iter()
        .enumerate()
        .filter(|(_, f)| alone(&chars, f.start, f.end))
        .collect();
    if standing.is_empty() {
        return (BTreeMap::new(), true);
    }
    standing.sort_by_key(|(_, f)| f.start);
    let piece = |from: usize, to: usize| -> String { chars.get(from..to).unwrap_or_default().iter().collect() };
    let mut pieces: Vec<(Option<usize>, String)> = Vec::new();
    let mut at = 0;
    for (k, fact) in standing {
        pieces.push((None, piece(at, fact.start)));
        pieces.push((Some(k), piece(fact.start, fact.end)));
        at = fact.end;
    }
    pieces.push((None, piece(at, chars.len())));
    let mut expected: Vec<Made> = Vec::new();
    let mut owners: Vec<Option<usize>> = Vec::new();
    for (owner, text) in &pieces {
        // a share the whole text was read with: what the reader refuses there it refused
        // in the whole (the call would have failed), so a refusal here makes nothing
        let made = if text.chars().all(py_space) {
            Vec::new()
        } else {
            markdown_shape(text).unwrap_or_default()
        };
        if made.first() == Some(&Made::Table) && expected.last() == Some(&Made::Table) {
            expected.push(Made::Paragraph);
            owners.push(None);
        }
        owners.extend(std::iter::repeat_n(*owner, made.len()));
        expected.extend(made);
    }
    let kinds: Vec<Option<char>> = blocks
        .iter()
        .map(|b| b.rsplit('/').next().and_then(|last| last.chars().next()))
        .collect();
    let want: Vec<Option<char>> = expected
        .iter()
        .map(|m| Some(if *m == Made::Table { 't' } else { 'p' }))
        .collect();
    for offset in [0usize, 1] {
        let window = kinds.get(offset..).unwrap_or_default();
        let window = window.get(..want.len().min(window.len())).unwrap_or_default();
        let extra = kinds.len().checked_sub(offset + want.len());
        if window == want.as_slice() && extra.is_some_and(|e| e <= 1) {
            let mut out: BTreeMap<usize, Vec<String>> = BTreeMap::new();
            for (owner, block) in owners.iter().zip(blocks.iter().skip(offset)) {
                if let Some(k) = owner {
                    out.entry(*k).or_default().push(block.clone());
                }
            }
            return (out, true);
        }
    }
    (BTreeMap::new(), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map};

    fn fact(markdown: &str, value: &str, own: Option<&str>) -> Written {
        let start = markdown.find(value).map(|b| markdown[..b].chars().count()).unwrap();
        let brief = json!({"from": "d:x/#1/a", "text": value});
        Written {
            field: "markdown".into(),
            start,
            end: start + value.chars().count(),
            text: value.into(),
            record: Map::new(),
            brief: brief.as_object().cloned().unwrap(),
            markdown: own.map(str::to_owned),
        }
    }

    #[test]
    fn alone_between_blank_lines() {
        let text: Vec<char> = "## A\n\nvalue\n \t\nafter".chars().collect();
        assert!(alone(&text, 6, 11));
        let text: Vec<char> = "Rated value today".chars().collect();
        assert!(!alone(&text, 6, 11));
        let text: Vec<char> = "  value  ".chars().collect();
        assert!(alone(&text, 2, 7));
    }

    #[test]
    fn plain_text_escaped_as_inline_markdown() {
        assert_eq!(
            plain_inline("a *b* & c &amp; [d]", false),
            "a \\*b\\* & c \\&amp; \\[d\\]"
        );
        assert_eq!(plain_inline("- item", true), "\\- item");
        assert_eq!(plain_inline("12. done", true), "12\\. done");
        assert_eq!(plain_inline("## two", true), "\\## two");
        assert_eq!(plain_inline("####### seven", true), "####### seven");
        assert_eq!(plain_inline("- item", false), "- item");
        assert_eq!(one_line(" one\n\n  two \r\nthree"), "one two three");
    }

    #[test]
    fn values_with_markdown_rewritten() {
        let markdown = "Intro\n\nline one\nline two\n\nsee line one\nline two here";
        let alone_fact = fact(markdown, "line one\nline two", Some("- **one**\n- two"));
        let mut inline = fact(markdown, "line one\nline two here", Some("x"));
        inline.end -= " here".chars().count();
        inline.text = "line one\nline two".into();
        let (out, placed) = with_markdown(markdown, &[&inline, &alone_fact]);
        assert_eq!(out, "Intro\n\n- **one**\n- two\n\nsee line one line two here");
        assert_eq!(placed[0].text, "- **one**\n- two");
        assert_eq!(placed[1].text, "line one line two");
        assert_eq!(
            &out.chars().collect::<Vec<_>>()[placed[1].start..placed[1].end]
                .iter()
                .collect::<String>(),
            "line one line two"
        );
    }

    #[test]
    fn blocks_lined_up() {
        let markdown = "## Doors\n\nfirst value\n\n## Roof\n\n- a\n- b";
        let facts = [fact(markdown, "first value", None), fact(markdown, "- a\n- b", None)];
        let blocks: Vec<String> = ["p:1", "p:2", "p:3", "p:4", "p:5"].map(String::from).to_vec();
        let (found, lined_up) = placed_blocks(markdown, &facts, &blocks);
        assert!(lined_up);
        assert_eq!(found.get(&0).unwrap(), &["p:2"]);
        assert_eq!(found.get(&1).unwrap(), &["p:4", "p:5"]);
        let (found, lined_up) = placed_blocks(markdown, &facts, &blocks[..3]);
        assert!(!lined_up && found.is_empty());
    }
}
