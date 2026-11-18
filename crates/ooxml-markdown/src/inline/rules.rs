//! Owns: the inline rules that read at a position (`rules_inline/`): `text` (up to the
//! next character some rule may read), `escape` (a backslash), `emphasis` (a run of `*`
//! or `_`, its flanking as CommonMark defines it).
//!
//! Must not: pair delimiters (that is [`super::pairs`]).

use super::{Delimiter, State};
use crate::chars::{is_punctuation, is_str_space, is_white_space};
use crate::token::Kind;

/// markdown-it's default terminators: where the `text` rule stops.
fn is_terminator(ch: char) -> bool {
    matches!(
        ch,
        '\n' | '!'
            | '#'
            | '$'
            | '%'
            | '&'
            | '*'
            | '+'
            | '-'
            | ':'
            | '<'
            | '='
            | '>'
            | '@'
            | '['
            | '\\'
            | ']'
            | '^'
            | '_'
            | '`'
            | '{'
            | '}'
            | '~'
    )
}

/// Text up to the next terminator, as pending text.
pub(super) fn text(state: &mut State<'_, '_>, silent: bool) -> bool {
    let start = state.pos;
    let mut pos = start;
    while state.at(pos).is_some_and(|ch| !is_terminator(ch)) {
        pos += 1;
    }
    if pos == start {
        return false;
    }
    if !silent {
        let text = state.slice(start, pos);
        state.append_pending(&text);
    }
    state.pos = pos;
    true
}

/// A backslash: before a newline a hard break, before ASCII punctuation that character,
/// before anything else itself and that character.
pub(super) fn escape(state: &mut State<'_, '_>, silent: bool) -> bool {
    let mut pos = state.pos;
    if state.at(pos) != Some('\\') {
        return false;
    }
    pos += 1;
    if pos >= state.pos_max {
        return false;
    }
    let Some(next) = state.at(pos) else { return false };
    if next == '\n' {
        if !silent {
            state.push(Kind::Hardbreak, "br", 0);
        }
        pos += 1;
        while pos < state.pos_max && state.at(pos).is_some_and(is_str_space) {
            pos += 1;
        }
        state.pos = pos;
        return true;
    }
    if !silent {
        let original = format!("\\{next}");
        let index = state.push(Kind::TextSpecial, "", 0);
        if let Some(token) = state.tokens.get_mut(index) {
            token.content = if next.is_ascii_punctuation() {
                next.to_string()
            } else {
                original.clone()
            };
            token.markup = original;
            "escape".clone_into(&mut token.info);
        }
    }
    state.pos = pos + 1;
    true
}

/// Whether a run of markers can open and close, and its length (`scanDelims`).
pub(crate) struct Scanned {
    pub(crate) can_open: bool,
    pub(crate) can_close: bool,
    pub(crate) length: usize,
}

/// Scan the marker run at `start` (`can_split_word`: whether it may sit inside a word).
pub(crate) fn scan_delims(state: &State<'_, '_>, start: usize, can_split_word: bool) -> Scanned {
    let marker = state.at(start);
    let last = start.checked_sub(1).and_then(|k| state.at(k)).unwrap_or(' ');
    let mut pos = start;
    while pos < state.pos_max && state.at(pos) == marker {
        pos += 1;
    }
    let next = if pos < state.pos_max {
        state.at(pos).unwrap_or(' ')
    } else {
        ' '
    };
    let (last_punct, next_punct) = (is_punctuation(last), is_punctuation(next));
    let (last_space, next_space) = (is_white_space(last), is_white_space(next));
    let left_flanking = !(next_space || (next_punct && !(last_space || last_punct)));
    let right_flanking = !(last_space || (last_punct && !(next_space || next_punct)));
    Scanned {
        can_open: left_flanking && (can_split_word || !right_flanking || last_punct),
        can_close: right_flanking && (can_split_word || !left_flanking || next_punct),
        length: pos - start,
    }
}

/// A run of `*` or `_`: each marker a text token and a delimiter.
pub(super) fn emphasis(state: &mut State<'_, '_>, silent: bool) -> bool {
    if silent {
        return false;
    }
    let Some(marker) = state.at(state.pos).filter(|m| matches!(m, '*' | '_')) else {
        return false;
    };
    let scanned = scan_delims(state, state.pos, marker == '*');
    for _ in 0..scanned.length {
        let token = state.push(Kind::Text, "", 0);
        if let Some(t) = state.tokens.get_mut(token) {
            t.content = marker.to_string();
        }
        let delimiter = Delimiter {
            marker,
            length: scanned.length,
            token,
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

/// A line end: a hard break after two spaces, else a soft break; the next line's
/// leading spaces skipped.
pub(super) fn newline(state: &mut State<'_, '_>, silent: bool) -> bool {
    let mut pos = state.pos;
    if state.at(pos) != Some('\n') {
        return false;
    }
    if !silent {
        let pending = state.pending_mut();
        let kind = if pending.ends_with("  ") {
            let kept = pending.trim_end_matches(' ').len();
            pending.truncate(kept);
            (Kind::Hardbreak, "br")
        } else {
            if pending.ends_with(' ') {
                pending.pop();
            }
            (Kind::Softbreak, "br")
        };
        state.push(kind.0, kind.1, 0);
    }
    pos += 1;
    while pos < state.pos_max && state.at(pos).is_some_and(is_str_space) {
        pos += 1;
    }
    state.pos = pos;
    true
}
