//! Owns: the block quote rule (`rules_block/blockquote.py`): lines starting `>` (and lazy
//! continuation lines), their marker and one space taken off, the blocks inside read with
//! the lines' measures changed and put back after.
//!
//! Must not: read inline content.

use super::{terminates, tokenize, Chain, Parent, State};
use crate::token::Kind;

/// A line's measures before the quote changed them.
struct Saved {
    b: usize,
    t: usize,
    s: isize,
    bs: isize,
}

/// After a `>` at `pos` of `line`: take the marker's space (a tab partly), and the
/// indent after it; whether the line is empty after the marker.
fn take_marker(state: &mut State<'_, '_>, line: usize, mut pos: usize) -> bool {
    let maximum = state.e(line);
    let mut initial = state.s(line) + 1;
    let mut offset = initial;
    let mut adjust_tab = false;
    let space_after = match state.at(pos) {
        Some(' ') => {
            pos += 1;
            initial += 1;
            offset += 1;
            true
        }
        Some('\t') => {
            if (state.bs(line) + offset) % 4 == 3 {
                pos += 1;
                initial += 1;
                offset += 1;
            } else {
                adjust_tab = true;
            }
            true
        }
        _ => false,
    };
    state.set_b(line, pos);
    while pos < maximum {
        match state.at(pos) {
            Some('\t') => offset += 4 - (offset + state.bs(line) + isize::from(adjust_tab)) % 4,
            Some(' ') => offset += 1,
            _ => break,
        }
        pos += 1;
    }
    let empty = pos >= maximum;
    state.set_bs(line, state.s(line) + 1 + isize::from(space_after));
    state.set_s(line, offset - initial);
    state.set_t(line, pos - state.b(line));
    empty
}

fn save(state: &State<'_, '_>, line: usize) -> Saved {
    Saved {
        b: state.b(line),
        t: state.t(line),
        s: state.s(line),
        bs: state.bs(line),
    }
}

pub(super) fn blockquote(state: &mut State<'_, '_>, start: usize, end: usize, silent: bool) -> bool {
    let old_line_max = state.line_max;
    let pos = state.first(start);
    if state.is_code_block(start) || state.at(pos) != Some('>') {
        return false;
    }
    if silent {
        return true;
    }
    let mut saved = vec![save(state, start)];
    let mut last_empty = take_marker(state, start, pos + 1);
    let old_parent = state.parent;
    state.parent = Parent::Blockquote;
    let mut next = start + 1;
    while next < end {
        let outdented = state.s(next) < state.blk_indent;
        let pos = state.first(next);
        let maximum = state.e(next);
        if pos >= maximum {
            break;
        }
        if state.at(pos) == Some('>') && !outdented {
            saved.push(save(state, next));
            last_empty = take_marker(state, next, pos + 1);
            next += 1;
            continue;
        }
        if last_empty {
            break;
        }
        if terminates(state, Chain::Blockquote, next, end) {
            state.line_max = next;
            if state.blk_indent != 0 {
                saved.push(save(state, next));
                state.set_s(next, state.s(next) - state.blk_indent);
            }
            break;
        }
        saved.push(save(state, next));
        state.set_s(next, -1);
        next += 1;
    }
    let old_indent = state.blk_indent;
    state.blk_indent = 0;
    let open = state.push(Kind::BlockquoteOpen, "blockquote", 1);
    if let Some(token) = state.token(open) {
        token.markup = ">".to_owned();
    }
    tokenize(state, start, next);
    let close = state.push(Kind::BlockquoteClose, "blockquote", -1);
    if let Some(token) = state.token(close) {
        token.markup = ">".to_owned();
    }
    state.line_max = old_line_max;
    state.parent = old_parent;
    let line = state.line;
    if let Some(token) = state.token(open) {
        token.map = Some((start, line));
    }
    for (k, old) in saved.iter().enumerate() {
        let line = start + k;
        state.set_b(line, old.b);
        state.set_t(line, old.t);
        state.set_s(line, old.s);
        state.set_bs(line, old.bs);
    }
    state.blk_indent = old_indent;
    true
}
