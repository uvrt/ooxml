//! Owns: the inline rules for spans and literals (`rules_inline/`): code spans
//! (`backticks`), `~~` markers (`strikethrough`), inline HTML (`html_inline`) and
//! character references (`entity`).
//!
//! Must not: pair delimiters (that is [`super::pairs`]).

use super::rules::scan_delims;
use super::{Delimiter, State};
use crate::link::{entity as named, valid_code};
use crate::token::Kind;
use std::collections::HashMap;

/// Each backtick run length's last start in `src`.
fn last_runs(src: &[char]) -> HashMap<usize, usize> {
    let mut out = HashMap::new();
    let mut pos = 0;
    while let Some(start) = src
        .get(pos..)
        .and_then(|r| r.iter().position(|c| *c == '`'))
        .map(|k| pos + k)
    {
        pos = start + 1;
        while src.get(pos) == Some(&'`') {
            pos += 1;
        }
        out.insert(pos - start, start);
    }
    out
}

/// A code span, or an unmatched backtick run as text.
pub(super) fn backticks(state: &mut State<'_, '_>, silent: bool) -> bool {
    let start = state.pos;
    if state.at(start) != Some('`') {
        return false;
    }
    let maximum = state.pos_max;
    let mut pos = start + 1;
    while pos < maximum && state.at(pos) == Some('`') {
        pos += 1;
    }
    let marker = state.slice(start, pos);
    let length = pos - start;
    let src = state.src;
    let runs = state.backticks.get_or_insert_with(|| last_runs(src));
    if runs.get(&length).is_some_and(|last| *last >= pos) {
        let mut match_end = pos;
        while let Some(match_start) = src
            .get(match_end..)
            .and_then(|r| r.iter().position(|c| *c == '`'))
            .map(|k| match_end + k)
        {
            if match_start >= maximum {
                break;
            }
            match_end = match_start + 1;
            while src.get(match_end) == Some(&'`') {
                match_end += 1;
            }
            if match_end > maximum {
                break;
            }
            if match_end - match_start == length {
                if !silent {
                    let mut content = state.slice(pos, match_start).replace('\n', " ");
                    if content.starts_with(' ') && content.ends_with(' ') && content.chars().any(|c| c != ' ') {
                        content = content.chars().skip(1).take(content.chars().count() - 2).collect();
                    }
                    let index = state.push(Kind::CodeInline, "code", 0);
                    if let Some(token) = state.tokens.get_mut(index) {
                        token.markup = marker;
                        token.content = content;
                    }
                }
                state.pos = match_end;
                return true;
            }
        }
    }
    if !silent {
        state.append_pending(&marker);
    }
    state.pos = pos;
    true
}

/// A run of two or more `~`: pairs of markers as text tokens and delimiters.
pub(super) fn strikethrough(state: &mut State<'_, '_>, silent: bool) -> bool {
    if silent || state.at(state.pos) != Some('~') {
        return false;
    }
    let scanned = scan_delims(state, state.pos, true);
    let mut length = scanned.length;
    if length < 2 {
        return false;
    }
    if length % 2 == 1 {
        let index = state.push(Kind::Text, "", 0);
        if let Some(token) = state.tokens.get_mut(index) {
            token.content = "~".to_owned();
        }
        length -= 1;
    }
    for _ in 0..length / 2 {
        let index = state.push(Kind::Text, "", 0);
        if let Some(token) = state.tokens.get_mut(index) {
            token.content = "~~".to_owned();
        }
        let delimiter = Delimiter {
            marker: '~',
            length: 0,
            token: index,
            end: None,
            open: scanned.can_open,
            close: scanned.can_close,
        };
        if let Some(list) = state.delimiters() {
            list.push(delimiter);
        }
    }
    state.pos += scanned.length;
    true
}

/// Inline HTML: a tag, comment, instruction, declaration or CDATA section.
pub(super) fn html_inline(state: &mut State<'_, '_>, silent: bool) -> bool {
    let pos = state.pos;
    if state.at(pos) != Some('<') || pos + 2 >= state.pos_max {
        return false;
    }
    let Some(ch) = state.at(pos + 1) else { return false };
    if !matches!(ch, '!' | '?' | '/') && !ch.is_ascii_alphabetic() {
        return false;
    }
    let starts = |s: &str| {
        state
            .src
            .get(pos..)
            .is_some_and(|r| r.iter().copied().take(s.chars().count()).eq(s.chars()))
    };
    let (comment, cdata) = (starts("<!--"), starts("<![CDATA["));
    let short_comment = starts("<!-->") || starts("<!--->");
    let before = |last: Option<usize>, at: usize| last.is_none_or(|l| l < at);
    let refused = if comment {
        before(state.last_of(">"), pos + 4) || (!short_comment && before(state.last_of("-->"), pos + 4))
    } else if cdata {
        before(state.last_of("]]>"), pos + 9)
    } else if ch == '?' {
        before(state.last_of("?>"), pos + 2)
    } else if ch == '!' {
        before(state.last_of(">"), pos + 3)
    } else {
        before(state.last_of(">"), pos + 2)
    };
    if refused {
        return false;
    }
    let Some(end) = crate::html::tag(state.src, pos) else {
        return false;
    };
    if !silent {
        let content = state.slice(pos, end);
        let index = state.push(Kind::HtmlInline, "", 0);
        if let Some(token) = state.tokens.get_mut(index) {
            token.content = content;
        }
    }
    state.pos = end;
    true
}

/// A character reference: a named one markdown-it knows, or a numeric one.
pub(super) fn entity(state: &mut State<'_, '_>, silent: bool) -> bool {
    let pos = state.pos;
    if state.at(pos) != Some('&') || pos + 1 >= state.pos_max {
        return false;
    }
    let (content, end) = if state.at(pos + 1) == Some('#') {
        let hex = matches!(state.at(pos + 2), Some('x' | 'X'));
        let from = pos + if hex { 3 } else { 2 };
        let limit = if hex { 6 } else { 7 };
        let mut end = from;
        while end - from < limit
            && state
                .at(end)
                .is_some_and(|c| if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() })
        {
            end += 1;
        }
        if end == from || state.at(end) != Some(';') {
            return false;
        }
        let digits = state.slice(from, end);
        let code = u32::from_str_radix(&digits, if hex { 16 } else { 10 }).unwrap_or(0xFFFD);
        let ch = char::from_u32(code).filter(|_| valid_code(code)).unwrap_or('\u{fffd}');
        (ch.to_string(), end + 1)
    } else {
        if !state.at(pos + 1).is_some_and(|c| c.is_ascii_alphabetic()) {
            return false;
        }
        let mut end = pos + 2;
        while end - pos - 1 < 32 && state.at(end).is_some_and(|c| c.is_ascii_alphanumeric()) {
            end += 1;
        }
        if end - pos - 1 < 2 || state.at(end) != Some(';') {
            return false;
        }
        let Some(text) = named(&state.slice(pos + 1, end)) else {
            return false;
        };
        (text.to_owned(), end + 1)
    };
    if !silent {
        let markup = state.slice(pos, end);
        let index = state.push(Kind::TextSpecial, "", 0);
        if let Some(token) = state.tokens.get_mut(index) {
            token.content = content;
            token.markup = markup;
            "entity".clone_into(&mut token.info);
        }
    }
    state.pos = end;
    true
}
