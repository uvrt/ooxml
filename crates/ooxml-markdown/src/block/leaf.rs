//! Owns: the leaf block rules (`rules_block/`): an indented code block (`code`), a
//! thematic break (`hr`), an ATX heading (`heading`), a setext heading (`lheading`) and a
//! paragraph (`paragraph`, which reads any line).
//!
//! Must not: read inline content.

use super::{terminates, Chain, Parent, State};
use crate::chars::{is_str_space, md_trim};
use crate::token::Kind;

/// Lines indented four columns past the block's indent.
pub(super) fn code(state: &mut State<'_, '_>, start: usize, end: usize, _silent: bool) -> bool {
    if !state.is_code_block(start) {
        return false;
    }
    let mut next = start + 1;
    let mut last = next;
    while next < end {
        if state.is_empty(next) {
            next += 1;
            continue;
        }
        if state.is_code_block(next) {
            next += 1;
            last = next;
            continue;
        }
        break;
    }
    state.line = last;
    let content = state.get_lines(start, last, 4 + state.blk_indent, false) + "\n";
    let index = state.push(Kind::CodeBlock, "code", 0);
    let line = state.line;
    if let Some(token) = state.token(index) {
        token.content = content;
        token.map = Some((start, line));
    }
    true
}

/// Three or more `*`, `-` or `_`, spaces between.
pub(super) fn hr(state: &mut State<'_, '_>, start: usize, _end: usize, silent: bool) -> bool {
    let mut pos = state.first(start);
    let maximum = state.e(start);
    if state.is_code_block(start) {
        return false;
    }
    let Some(marker) = state.at(pos).filter(|m| matches!(m, '*' | '-' | '_')) else {
        return false;
    };
    pos += 1;
    let mut count = 1;
    while pos < maximum {
        let ch = state.at(pos);
        pos += 1;
        if ch != Some(marker) && !ch.is_some_and(is_str_space) {
            return false;
        }
        if ch == Some(marker) {
            count += 1;
        }
    }
    if count < 3 {
        return false;
    }
    if silent {
        return true;
    }
    state.line = start + 1;
    let index = state.push(Kind::Hr, "hr", 0);
    let line = state.line;
    if let Some(token) = state.token(index) {
        token.map = Some((start, line));
        token.markup = std::iter::repeat_n(marker, count + 1).collect();
    }
    true
}

/// `#` to `######`, a space, the heading's text, closing `#`s left out.
pub(super) fn heading(state: &mut State<'_, '_>, start: usize, _end: usize, silent: bool) -> bool {
    let mut pos = state.first(start);
    let mut maximum = state.e(start);
    if state.is_code_block(start) || state.at(pos) != Some('#') || pos >= maximum {
        return false;
    }
    let mut level = 1;
    pos += 1;
    while state.at(pos) == Some('#') && pos < maximum && level <= 6 {
        level += 1;
        pos += 1;
    }
    if level > 6 || (pos < maximum && !state.at(pos).is_some_and(is_str_space)) {
        return false;
    }
    if silent {
        return true;
    }
    maximum = state.skip_spaces_back(maximum, pos);
    let tmp = state.skip_chars_back(maximum, '#', pos);
    if tmp > pos && state.at(tmp - 1).is_some_and(is_str_space) {
        maximum = tmp;
    }
    state.line = start + 1;
    let tag = format!("h{level}");
    let markup: String = std::iter::repeat_n('#', level).collect();
    let content = md_trim(&state.slice(pos, maximum)).to_owned();
    push_heading(state, &tag, &markup, content, (start, state.line, state.line));
    true
}

/// A heading's three tokens; `lines`: the heading's first line, its end, the text's end.
fn push_heading(state: &mut State<'_, '_>, tag: &str, markup: &str, content: String, lines: (usize, usize, usize)) {
    let open = state.push(Kind::HeadingOpen, tag, 1);
    if let Some(token) = state.token(open) {
        markup.clone_into(&mut token.markup);
        token.map = Some((lines.0, lines.1));
    }
    let inline = state.push(Kind::Inline, "", 0);
    if let Some(token) = state.token(inline) {
        token.content = content;
        token.map = Some((lines.0, lines.2));
        token.children = Some(Vec::new());
    }
    let close = state.push(Kind::HeadingClose, tag, -1);
    if let Some(token) = state.token(close) {
        markup.clone_into(&mut token.markup);
    }
}

/// A paragraph underlined with `=` (level 1) or `-` (level 2).
pub(super) fn lheading(state: &mut State<'_, '_>, start: usize, end: usize, _silent: bool) -> bool {
    if state.is_code_block(start) {
        return false;
    }
    let mut next = start + 1;
    let old_parent = state.parent;
    state.parent = Parent::Paragraph;
    let mut found: Option<(usize, char)> = None;
    while next < end && !state.is_empty(next) {
        if state.s(next) - state.blk_indent > 3 {
            next += 1;
            continue;
        }
        if state.s(next) >= state.blk_indent {
            let mut pos = state.first(next);
            let maximum = state.e(next);
            if pos < maximum {
                if let Some(marker) = state.at(pos).filter(|m| matches!(m, '-' | '=')) {
                    pos = state.skip_chars(pos, marker);
                    pos = state.skip_spaces(pos);
                    if pos >= maximum {
                        found = Some((if marker == '=' { 1 } else { 2 }, marker));
                        break;
                    }
                }
            }
        }
        if state.s(next) < 0 {
            next += 1;
            continue;
        }
        if terminates(state, Chain::Paragraph, next, end) {
            break;
        }
        next += 1;
    }
    let Some((level, marker)) = found else {
        // markdown-it leaves the parent kind a paragraph here (its JavaScript source too)
        return false;
    };
    let content = md_trim(&state.get_lines(start, next, state.blk_indent, false)).to_owned();
    state.line = next + 1;
    let line = state.line;
    push_heading(
        state,
        &format!("h{level}"),
        &marker.to_string(),
        content,
        (start, line, line - 1),
    );
    state.parent = old_parent;
    true
}

/// Lines up to an empty one or the start of another block.
pub(super) fn paragraph(state: &mut State<'_, '_>, start: usize, _end: usize, _silent: bool) -> bool {
    let mut next = start + 1;
    let end = state.line_max;
    let old_parent = state.parent;
    state.parent = Parent::Paragraph;
    while next < end {
        if state.is_empty(next) {
            break;
        }
        if state.s(next) - state.blk_indent > 3 || state.s(next) < 0 {
            next += 1;
            continue;
        }
        if terminates(state, Chain::Paragraph, next, end) {
            break;
        }
        next += 1;
    }
    let content = md_trim(&state.get_lines(start, next, state.blk_indent, false)).to_owned();
    state.line = next;
    let open = state.push(Kind::ParagraphOpen, "p", 1);
    let line = state.line;
    if let Some(token) = state.token(open) {
        token.map = Some((start, line));
    }
    let inline = state.push(Kind::Inline, "", 0);
    if let Some(token) = state.token(inline) {
        token.content = content;
        token.map = Some((start, line));
        token.children = Some(Vec::new());
    }
    state.push(Kind::ParagraphClose, "p", -1);
    state.parent = old_parent;
    true
}
