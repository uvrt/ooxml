//! Owns: markdown-it's post-processing of a text's tokens (`rules_inline/balance_pairs.py`,
//! `emphasis.py`'s `postProcess`, `fragments_join.py`): each closing marker paired with
//! its opener (CommonMark's "rule of 3"), the pairs turned into `<em>`/`<strong>`, adjacent
//! text joined.
//!
//! Must not: read the source text.

use super::{Delimiter, State};
use crate::token::{Kind, Token};
use std::collections::HashMap;

/// Pair the delimiters of every level (`link_pairs`).
pub(super) fn balance_pairs(state: &mut State<'_, '_>) {
    for list in &mut state.lists {
        process_delimiters(list);
    }
}

/// For each closing marker find its opener (`processDelimiters`).
fn process_delimiters(delimiters: &mut [Delimiter]) {
    if delimiters.is_empty() {
        return;
    }
    let mut openers_bottom: HashMap<char, [isize; 6]> = HashMap::new();
    let mut header: usize = 0;
    let mut last_token: isize = -2;
    let mut jumps: Vec<usize> = Vec::with_capacity(delimiters.len());
    for closer_at in 0..delimiters.len() {
        jumps.push(0);
        let Some(closer) = delimiters.get(closer_at).copied() else {
            break;
        };
        let header_marker = delimiters.get(header).map(|d| d.marker);
        if header_marker != Some(closer.marker) || last_token != signed(closer.token) - 1 {
            header = closer_at;
        }
        last_token = signed(closer.token);
        if !closer.close {
            continue;
        }
        let slot = usize::from(closer.open) * 3 + closer.length % 3;
        let bottom = openers_bottom.entry(closer.marker).or_insert([-1; 6]);
        let min_opener = bottom.get(slot).copied().unwrap_or(-1);
        let mut opener_at = signed(header) - signed(jump(&jumps, header)) - 1;
        let mut new_min = opener_at;
        while opener_at > min_opener {
            let Some(opener) = index(opener_at).and_then(|k| delimiters.get(k)).copied() else {
                break;
            };
            let at = index(opener_at).unwrap_or(0);
            if opener.marker == closer.marker && opener.open && opener.end.is_none() {
                let odd = (opener.close || closer.open)
                    && (opener.length + closer.length) % 3 == 0
                    && (opener.length % 3 != 0 || closer.length % 3 != 0);
                if !odd {
                    let last_jump = match at.checked_sub(1).and_then(|k| delimiters.get(k)) {
                        Some(before) if !before.open => jump(&jumps, at - 1) + 1,
                        _ => 0,
                    };
                    if let Some(j) = jumps.get_mut(closer_at) {
                        *j = closer_at - at + last_jump;
                    }
                    if let Some(j) = jumps.get_mut(at) {
                        *j = last_jump;
                    }
                    if let Some(c) = delimiters.get_mut(closer_at) {
                        c.open = false;
                    }
                    if let Some(o) = delimiters.get_mut(at) {
                        o.end = Some(closer_at);
                        o.close = false;
                    }
                    new_min = -1;
                    last_token = -2;
                    break;
                }
            }
            opener_at -= signed(jump(&jumps, at)) + 1;
        }
        if new_min != -1 {
            let open = delimiters.get(closer_at).is_some_and(|c| c.open);
            let slot = usize::from(open) * 3 + closer.length % 3;
            if let Some(b) = openers_bottom.get_mut(&closer.marker).and_then(|b| b.get_mut(slot)) {
                *b = new_min;
            }
        }
    }
}

fn jump(jumps: &[usize], at: usize) -> usize {
    jumps.get(at).copied().unwrap_or(0)
}

fn signed(value: usize) -> isize {
    isize::try_from(value).unwrap_or(isize::MAX)
}

fn index(value: isize) -> Option<usize> {
    usize::try_from(value).ok()
}

/// Turn every level's paired `*`/`_` markers into emphasis (`emphasis.postProcess`).
pub(super) fn emphasis(state: &mut State<'_, '_>) {
    for list in &state.lists {
        emphasis_of(&mut state.tokens, list);
    }
}

fn emphasis_of(tokens: &mut [Token], delimiters: &[Delimiter]) {
    let mut i = delimiters.len();
    while i > 0 {
        i -= 1;
        let Some(start) = delimiters.get(i).copied() else { break };
        if !matches!(start.marker, '_' | '*') {
            continue;
        }
        let Some(end_at) = start.end else { continue };
        let Some(end) = delimiters.get(end_at).copied() else {
            continue;
        };
        let outer = i.checked_sub(1).and_then(|k| delimiters.get(k)).copied();
        let after = delimiters.get(end_at + 1).copied();
        let strong = match (outer, after) {
            (Some(o), Some(a)) => {
                o.end == Some(end_at + 1)
                    && o.marker == start.marker
                    && o.token + 1 == start.token
                    && a.token == end.token + 1
            }
            _ => false,
        };
        let marker: String = std::iter::repeat_n(start.marker, if strong { 2 } else { 1 }).collect();
        let (open, close, tag) = if strong {
            (Kind::StrongOpen, Kind::StrongClose, "strong")
        } else {
            (Kind::EmOpen, Kind::EmClose, "em")
        };
        if let Some(t) = tokens.get_mut(start.token) {
            *t = Token {
                kind: open,
                tag: tag.to_owned(),
                nesting: 1,
                markup: marker.clone(),
                content: String::new(),
                ..t.clone()
            };
        }
        if let Some(t) = tokens.get_mut(end.token) {
            *t = Token {
                kind: close,
                tag: tag.to_owned(),
                nesting: -1,
                markup: marker,
                content: String::new(),
                ..t.clone()
            };
        }
        if strong {
            for k in [outer.map(|o| o.token), after.map(|a| a.token)].into_iter().flatten() {
                if let Some(t) = tokens.get_mut(k) {
                    t.content.clear();
                }
            }
            i = i.saturating_sub(1);
        }
    }
}

/// Join adjacent text tokens and give every token its level (`fragments_join`).
pub(super) fn fragments_join(tokens: &mut Vec<Token>) {
    let mut level = 0;
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    for mut token in tokens.drain(..) {
        if token.nesting < 0 {
            level -= 1;
        }
        token.level = level;
        if token.nesting > 0 {
            level += 1;
        }
        match out.last_mut() {
            Some(last) if token.kind == Kind::Text && last.kind == Kind::Text => {
                last.content.push_str(&token.content);
                last.level = level;
            }
            _ => out.push(token),
        }
    }
    *tokens = out;
}

/// Turn every level's paired `~~` markers into strikethrough, lone `~` moved past the
/// closers (`strikethrough.postProcess`).
pub(super) fn strikethrough(state: &mut State<'_, '_>) {
    for list in &state.lists {
        strike_of(&mut state.tokens, list);
    }
}

fn strike_of(tokens: &mut [Token], delimiters: &[Delimiter]) {
    let mut lone: Vec<usize> = Vec::new();
    for start in delimiters {
        if start.marker != '~' {
            continue;
        }
        let Some(end) = start.end.and_then(|e| delimiters.get(e)) else {
            continue;
        };
        let markup = tokens.get(start.token).map(|t| t.content.clone()).unwrap_or_default();
        if let Some(t) = tokens.get_mut(start.token) {
            *t = Token {
                kind: Kind::SOpen,
                tag: "s".to_owned(),
                nesting: 1,
                markup: markup.clone(),
                content: String::new(),
                ..t.clone()
            };
        }
        if let Some(t) = tokens.get_mut(end.token) {
            *t = Token {
                kind: Kind::SClose,
                tag: "s".to_owned(),
                nesting: -1,
                markup,
                content: String::new(),
                ..t.clone()
            };
        }
        let before = end.token.checked_sub(1).and_then(|k| tokens.get(k));
        if before.is_some_and(|t| t.kind == Kind::Text && t.content == "~") {
            lone.push(end.token - 1);
        }
    }
    while let Some(i) = lone.pop() {
        let mut j = i + 1;
        while tokens.get(j).is_some_and(|t| t.kind == Kind::SClose) {
            j += 1;
        }
        j -= 1;
        if i != j {
            tokens.swap(i, j);
        }
    }
}
