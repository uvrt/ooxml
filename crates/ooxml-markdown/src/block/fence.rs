//! Owns: the fenced code block rule (`rules_block/fence.py`): three or more backticks or
//! tildes, an info string (no backtick after backticks), the lines up to a fence at least
//! as long.
//!
//! Must not: read inline content.

use super::State;
use crate::token::Kind;

pub(super) fn fence(state: &mut State<'_, '_>, start: usize, end: usize, silent: bool) -> bool {
    let mut pos = state.first(start);
    let mut maximum = state.e(start);
    if state.is_code_block(start) || pos + 3 > maximum {
        return false;
    }
    let Some(marker) = state.at(pos).filter(|m| matches!(m, '~' | '`')) else {
        return false;
    };
    let mut mem = pos;
    pos = state.skip_chars(pos, marker);
    let length = pos - mem;
    if length < 3 {
        return false;
    }
    let markup = state.slice(mem, pos);
    let params = state.slice(pos, maximum);
    if marker == '`' && params.contains('`') {
        return false;
    }
    if silent {
        return true;
    }
    let mut next = start;
    let mut closed = false;
    loop {
        next += 1;
        if next >= end {
            break;
        }
        pos = state.first(next);
        mem = pos;
        maximum = state.e(next);
        if pos < maximum && state.s(next) < state.blk_indent {
            break;
        }
        match state.at(pos) {
            None => break,
            Some(ch) if ch != marker => continue,
            Some(_) => {}
        }
        if state.is_code_block(next) {
            continue;
        }
        pos = state.skip_chars(pos, marker);
        if pos - mem < length {
            continue;
        }
        pos = state.skip_spaces(pos);
        if pos < maximum {
            continue;
        }
        closed = true;
        break;
    }
    let indent = state.s(start);
    state.line = next + usize::from(closed);
    let content = state.get_lines(start + 1, next, indent, true);
    let index = state.push(Kind::Fence, "code", 0);
    let line = state.line;
    if let Some(token) = state.token(index) {
        token.info = params;
        token.content = content;
        token.markup = markup;
        token.map = Some((start, line));
    }
    true
}
