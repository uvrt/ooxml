//! Owns: the list rule (`rules_block/list.py`): bullet (`*`, `-`, `+`) and ordered
//! (`1.`, `1)`) list items, each item's content indent, the blocks inside read with the
//! first line's measures changed and put back, tight lists' paragraphs hidden.
//!
//! Must not: read inline content.

use super::{terminates, tokenize, Chain, Parent, State};
use crate::chars::is_str_space;
use crate::token::{Kind, Value};

/// The place after a bullet marker and its space, if the line starts with one.
fn bullet_marker(state: &State<'_, '_>, line: usize) -> Option<usize> {
    let pos = state.first(line);
    let maximum = state.e(line);
    state.at(pos).filter(|m| matches!(m, '*' | '-' | '+'))?;
    let pos = pos + 1;
    if pos < maximum && !state.at(pos).is_some_and(is_str_space) {
        return None;
    }
    Some(pos)
}

/// The place after an ordered marker (up to nine digits, `.` or `)`) and its space.
fn ordered_marker(state: &State<'_, '_>, line: usize) -> Option<usize> {
    let start = state.first(line);
    let maximum = state.e(line);
    if start + 1 >= maximum {
        return None;
    }
    let mut pos = start;
    state.at(pos).filter(char::is_ascii_digit)?;
    pos += 1;
    loop {
        if pos >= maximum {
            return None;
        }
        let ch = state.at(pos)?;
        pos += 1;
        if ch.is_ascii_digit() {
            if pos - start >= 10 {
                return None;
            }
            continue;
        }
        if matches!(ch, ')' | '.') {
            break;
        }
        return None;
    }
    if pos < maximum && !state.at(pos).is_some_and(is_str_space) {
        return None;
    }
    Some(pos)
}

/// Hide the paragraphs directly in a tight list's items.
fn mark_tight_paragraphs(state: &mut State<'_, '_>, index: usize) {
    let level = state.level + 2;
    let length = state.tokens.len().saturating_sub(2);
    let mut i = index + 2;
    while i < length {
        let found = state
            .tokens
            .get(i)
            .is_some_and(|t| t.level == level && t.kind == Kind::ParagraphOpen);
        if found {
            for k in [i, i + 2] {
                if let Some(token) = state.tokens.get_mut(k) {
                    token.hidden = true;
                }
            }
            i += 2;
        }
        i += 1;
    }
}

/// One list item from `start`, its marker ending at `after`: its content indent, the
/// blocks inside read with the first line's measures changed and put back; the item's
/// open token, whether it kept the list tight, whether it ended with an empty line.
fn item(
    state: &mut State<'_, '_>,
    (start, end): (usize, usize),
    (after, marker, info): (usize, char, String),
    prev_empty_end: bool,
) -> (usize, bool, bool) {
    let mut pos = after;
    let maximum = state.e(start);
    let initial =
        state.s(start) + isize::try_from(after).unwrap_or(0) - isize::try_from(state.first(start)).unwrap_or(0);
    let mut offset = initial;
    while pos < maximum {
        match state.at(pos) {
            Some('\t') => offset += 4 - (offset + state.bs(start)) % 4,
            Some(' ') => offset += 1,
            _ => break,
        }
        pos += 1;
    }
    let content_start = pos;
    let mut indent_after = if content_start >= maximum { 1 } else { offset - initial };
    if indent_after > 4 {
        indent_after = 1;
    }
    let item = state.push(Kind::ListItemOpen, "li", 1);
    if let Some(token) = state.token(item) {
        token.markup = marker.to_string();
        token.info = info;
        token.map = Some((start, 0));
    }
    let (old_tight, old_t, old_s, old_list_indent) = (state.tight, state.t(start), state.s(start), state.list_indent);
    state.list_indent = state.blk_indent;
    state.blk_indent = initial + indent_after;
    state.tight = true;
    state.set_t(start, content_start - state.b(start));
    state.set_s(start, offset);
    if content_start >= maximum && state.is_empty(start + 1) {
        state.line = (state.line + 2).min(end);
    } else {
        tokenize(state, start, end);
    }
    let tight = state.tight && !prev_empty_end;
    let empty_end = state.line - start > 1 && state.line > 0 && state.is_empty(state.line - 1);
    state.blk_indent = state.list_indent;
    state.list_indent = old_list_indent;
    state.set_t(start, old_t);
    state.set_s(start, old_s);
    state.tight = old_tight;
    let close = state.push(Kind::ListItemClose, "li", -1);
    if let Some(token) = state.token(close) {
        token.markup = marker.to_string();
    }
    (item, tight, empty_end)
}

/// The list starting at `start`: whether it is ordered, the place after its marker,
/// the marker character and its number; `None` where no list may start (`silent`:
/// looking whether one ends a paragraph).
fn opening(state: &State<'_, '_>, start: usize, silent: bool) -> Option<(bool, usize, char, i64)> {
    if state.is_code_block(start)
        || (state.list_indent >= 0 && state.s(start) - state.list_indent >= 4 && state.s(start) < state.blk_indent)
    {
        return None;
    }
    let terminating = silent && state.parent == Parent::Paragraph && state.s(start) >= state.blk_indent;
    let marker_start = state.first(start);
    let (ordered, after) = match ordered_marker(state, start) {
        Some(after) => (true, after),
        None => (false, bullet_marker(state, start)?),
    };
    let value: i64 = if ordered {
        state.slice(marker_start, after - 1).parse().unwrap_or(0)
    } else {
        1
    };
    if terminating && ((ordered && value != 1) || state.skip_spaces(after) >= state.e(start)) {
        return None;
    }
    Some((ordered, after, state.at(after - 1)?, value))
}

pub(super) fn list(state: &mut State<'_, '_>, start: usize, end: usize, silent: bool) -> bool {
    let Some((ordered, mut after, marker, value)) = opening(state, start, silent) else {
        return false;
    };
    if silent {
        return true;
    }
    let mut marker_start = state.first(start);
    let list_index = if ordered {
        let i = state.push(Kind::OrderedListOpen, "ol", 1);
        if value != 1 {
            if let Some(token) = state.token(i) {
                token.set_attr("start", Value::Number(value));
            }
        }
        i
    } else {
        state.push(Kind::BulletListOpen, "ul", 1)
    };
    if let Some(token) = state.token(list_index) {
        token.markup = marker.to_string();
    }
    let first_line = start;
    let mut start = start;
    let mut next = start;
    let mut prev_empty_end = false;
    let mut tight = true;
    let old_parent = state.parent;
    state.parent = Parent::List;
    while next < end {
        let info = if ordered {
            state.slice(marker_start, after - 1)
        } else {
            String::new()
        };
        let (item, item_tight, empty_end) = item(state, (start, end), (after, marker, info), prev_empty_end);
        tight &= item_tight;
        prev_empty_end = empty_end;
        start = state.line;
        next = start;
        if let Some(token) = state.token(item) {
            token.map = Some((token.map.map_or(0, |m| m.0), next));
        }
        if next >= end || state.s(next) < state.blk_indent || state.is_code_block(start) {
            break;
        }
        if terminates(state, Chain::List, next, end) {
            break;
        }
        let found = if ordered {
            ordered_marker(state, next)
        } else {
            bullet_marker(state, next)
        };
        let Some(found) = found else { break };
        after = found;
        if ordered {
            marker_start = state.first(next);
        }
        if state.at(after - 1) != Some(marker) {
            break;
        }
    }
    let close = if ordered {
        state.push(Kind::OrderedListClose, "ol", -1)
    } else {
        state.push(Kind::BulletListClose, "ul", -1)
    };
    if let Some(token) = state.token(close) {
        token.markup = marker.to_string();
    }
    if let Some(token) = state.token(list_index) {
        token.map = Some((first_line, next));
    }
    state.line = next;
    state.parent = old_parent;
    if tight {
        mark_tight_paragraphs(state, list_index);
    }
    true
}
