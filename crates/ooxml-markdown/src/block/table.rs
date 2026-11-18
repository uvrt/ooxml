//! Owns: the GFM table rule (`rules_block/table.py`): a header row, a delimiter row giving
//! each column's alignment, body rows up to an empty line or another block; cells split at
//! unescaped pipes.
//!
//! Must not: read inline content.

use super::{terminates, Chain, Parent, State};
use crate::chars::{is_str_space, md_trim};
use crate::token::{Kind, Value};

/// markdown-it's cap on cells added to short rows.
const MAX_AUTOCOMPLETED_CELLS: isize = 0x10000;

fn line_text(state: &State<'_, '_>, line: usize) -> String {
    state.slice(state.first(line), state.e(line))
}

/// Split at `|` not after a backslash (the backslash dropped) (`escapedSplit`).
fn escaped_split(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let collect = |a: usize, b: usize| -> String { chars.get(a..b.max(a)).unwrap_or_default().iter().collect() };
    let mut out = Vec::new();
    let (mut last, mut current, mut escaped) = (0usize, String::new(), false);
    for (pos, ch) in chars.iter().enumerate() {
        if *ch == '|' {
            if escaped {
                current.push_str(&collect(last, pos - 1));
                last = pos;
            } else {
                out.push(std::mem::take(&mut current) + &collect(last, pos));
                last = pos + 1;
            }
        }
        escaped = *ch == '\\';
    }
    out.push(current + &collect(last, chars.len()));
    out
}

fn trim_edges(mut columns: Vec<String>) -> Vec<String> {
    if columns.first().is_some_and(String::is_empty) {
        columns.remove(0);
    }
    if columns.last().is_some_and(String::is_empty) {
        columns.pop();
    }
    columns
}

/// The delimiter row's alignments, if `line` is one.
fn aligns(state: &State<'_, '_>, line: usize) -> Option<Vec<&'static str>> {
    if state.s(line) < state.blk_indent || state.is_code_block(line) {
        return None;
    }
    let mut pos = state.first(line);
    let end = state.e(line);
    let marks = |c: char| matches!(c, '|' | '-' | ':');
    let first = state.at(pos).filter(|_| pos < end)?;
    pos += 1;
    if !marks(first) {
        return None;
    }
    let second = state.at(pos).filter(|_| pos < end)?;
    if !marks(second) && !is_str_space(second) || (first == '-' && is_str_space(second)) {
        return None;
    }
    pos += 1;
    while pos < end {
        let ch = state.at(pos)?;
        if !marks(ch) && !is_str_space(ch) {
            return None;
        }
        pos += 1;
    }
    let text = line_text(state, line);
    let columns: Vec<&str> = text.split('|').collect();
    let mut out = Vec::new();
    for (i, column) in columns.iter().enumerate() {
        let t = md_trim(column);
        if t.is_empty() {
            if i == 0 || i == columns.len() - 1 {
                continue;
            }
            return None;
        }
        let inner = t.strip_prefix(':').unwrap_or(t);
        let inner = inner.strip_suffix(':').unwrap_or(inner);
        if inner.is_empty() || !inner.chars().all(|c| c == '-') {
            return None;
        }
        out.push(match (t.starts_with(':'), t.ends_with(':')) {
            (true, true) => "center",
            (false, true) => "right",
            (true, false) => "left",
            (false, false) => "",
        });
    }
    Some(out)
}

fn push_cell(
    state: &mut State<'_, '_>,
    (open, close, tag): (Kind, Kind, &str),
    align: &str,
    content: String,
    line: usize,
) {
    let index = state.push(open, tag, 1);
    if !align.is_empty() {
        if let Some(token) = state.token(index) {
            token.set_attr("style", Value::Text(format!("text-align:{align}")));
        }
    }
    let inline = state.push(Kind::Inline, "", 0);
    if let Some(token) = state.token(inline) {
        token.map = Some((line, line + 1));
        token.content = content;
        token.children = Some(Vec::new());
    }
    state.push(close, tag, -1);
}

pub(super) fn table(state: &mut State<'_, '_>, start: usize, end: usize, silent: bool) -> bool {
    if start + 2 > end {
        return false;
    }
    let Some(aligns) = aligns(state, start + 1) else {
        return false;
    };
    let header = md_trim(&line_text(state, start)).to_owned();
    if !header.contains('|') || state.is_code_block(start) {
        return false;
    }
    let columns = trim_edges(escaped_split(&header));
    let count = columns.len();
    if count == 0 || count != aligns.len() {
        return false;
    }
    if silent {
        return true;
    }
    let old_parent = state.parent;
    state.parent = Parent::Table;
    let table = state.push(Kind::TableOpen, "table", 1);
    for (kind, tag) in [(Kind::TheadOpen, "thead"), (Kind::TrOpen, "tr")] {
        let index = state.push(kind, tag, 1);
        if let Some(token) = state.token(index) {
            token.map = Some((start, start + 1));
        }
    }
    for (i, column) in columns.iter().enumerate() {
        let align = aligns.get(i).copied().unwrap_or("");
        push_cell(
            state,
            (Kind::ThOpen, Kind::ThClose, "th"),
            align,
            md_trim(column).to_owned(),
            start,
        );
    }
    state.push(Kind::TrClose, "tr", -1);
    state.push(Kind::TheadClose, "thead", -1);
    let mut autocompleted: isize = 0;
    let mut next = start + 2;
    let mut body = None;
    while next < end {
        if state.s(next) < state.blk_indent || terminates(state, Chain::Blockquote, next, end) {
            break;
        }
        let text = md_trim(&line_text(state, next)).to_owned();
        if text.is_empty() || state.is_code_block(next) {
            break;
        }
        let cells = trim_edges(escaped_split(&text));
        autocompleted += isize::try_from(count).unwrap_or(0) - isize::try_from(cells.len()).unwrap_or(0);
        if autocompleted > MAX_AUTOCOMPLETED_CELLS {
            break;
        }
        if next == start + 2 {
            body = Some(state.push(Kind::TbodyOpen, "tbody", 1));
        }
        let row = state.push(Kind::TrOpen, "tr", 1);
        if let Some(token) = state.token(row) {
            token.map = Some((next, next + 1));
        }
        for i in 0..count {
            let content = cells.get(i).map(|c| md_trim(c).to_owned()).unwrap_or_default();
            let align = aligns.get(i).copied().unwrap_or("");
            push_cell(state, (Kind::TdOpen, Kind::TdClose, "td"), align, content, next);
        }
        state.push(Kind::TrClose, "tr", -1);
        next += 1;
    }
    if let Some(index) = body {
        state.push(Kind::TbodyClose, "tbody", -1);
        if let Some(token) = state.token(index) {
            token.map = Some((start + 2, next));
        }
    }
    state.push(Kind::TableClose, "table", -1);
    if let Some(token) = state.token(table) {
        token.map = Some((start, next));
    }
    state.parent = old_parent;
    state.line = next;
    true
}
