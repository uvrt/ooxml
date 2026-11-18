//! Owns: the inline rules that read a label (`rules_inline/link.py`, `image.py`,
//! `autolink.py`, markdown-it-footnote's `footnote_inline` and `footnote_ref`): a link's
//! or image's label, destination and title (or a reference definition's), an autolink, a
//! footnote's reference or inline footnote.
//!
//! Must not: pair delimiters (that is [`super::pairs`]).

use super::{skip_token, tokenize, State};
use crate::chars::is_str_space;
use crate::link::{destination, normalize_reference, title};
use crate::token::{Kind, Note, Value};
use crate::url::{normalize_link, normalize_link_text, validate_link};

/// The end of the label whose `[` is at `start` (`parseLinkLabel`); `disable_nested`: a
/// label holding another fails.
fn link_label(state: &mut State<'_, '_>, start: usize, disable_nested: bool) -> Option<usize> {
    let old = state.pos;
    state.pos = start + 1;
    let mut level = 1;
    let mut found = false;
    while state.pos < state.pos_max {
        let marker = state.at(state.pos);
        if marker == Some(']') {
            level -= 1;
            if level == 0 {
                found = true;
                break;
            }
        }
        let previous = state.pos;
        skip_token(state);
        if marker == Some('[') {
            if previous == state.pos - 1 {
                level += 1;
            } else if disable_nested {
                state.pos = old;
                return None;
            }
        }
    }
    let end = found.then_some(state.pos);
    state.pos = old;
    end
}

fn skip_space(state: &State<'_, '_>, mut pos: usize) -> usize {
    while pos < state.pos_max && state.at(pos).is_some_and(|c| is_str_space(c) || c == '\n') {
        pos += 1;
    }
    pos
}

/// What follows a label: `(destination "title")` inline, else a reference.
struct Target {
    href: String,
    title: String,
    pos: usize,
}

/// `(dest "title")` at `pos` (just after the label): the target, or `Err(reference)`
/// when markdown-it falls back to reading a reference; `None` to fail.
fn inline_target(state: &State<'_, '_>, mut pos: usize, image: bool) -> Option<Result<Target, ()>> {
    let maximum = state.pos_max;
    if !(pos < maximum && state.at(pos) == Some('(')) {
        return Some(Err(()));
    }
    pos = skip_space(state, pos + 1);
    if pos >= maximum {
        return None;
    }
    let mut href = String::new();
    let mut found_title = String::new();
    let dest = destination(state.src, pos, maximum);
    if let Some(dest) = &dest {
        href = normalize_link(&dest.text);
        if validate_link(&href) {
            pos = dest.pos;
        } else {
            href.clear();
        }
    }
    if dest.is_some() || image {
        let start = pos;
        pos = skip_space(state, pos);
        let read = title(state.src, pos, maximum, None);
        if pos < maximum && start != pos && read.ok {
            found_title = read.text;
            pos = skip_space(state, read.pos);
        }
    }
    if pos >= maximum || state.at(pos) != Some(')') {
        return if image { None } else { Some(Err(())) };
    }
    Some(Ok(Target {
        href,
        title: found_title,
        pos: pos + 1,
    }))
}

/// A reference after the label `[label_start, label_end)`: `[ref]`, `[]` or nothing.
fn reference_target(state: &mut State<'_, '_>, label_start: usize, label_end: usize, from: usize) -> Option<Target> {
    if state.env.references.is_empty() {
        return None;
    }
    let mut pos = from;
    let mut label = String::new();
    if pos < state.pos_max && state.at(pos) == Some('[') {
        let start = pos + 1;
        match link_label(state, pos, false) {
            Some(end) => {
                label = state.slice(start, end);
                pos = end + 1;
            }
            None => pos = label_end + 1,
        }
    } else {
        pos = label_end + 1;
    }
    if label.is_empty() {
        label = state.slice(label_start, label_end);
    }
    let found = state.env.references.get(&normalize_reference(&label))?;
    Some(Target {
        href: found.href.clone(),
        title: found.title.clone(),
        pos,
    })
}

/// `[text](dest "title")` or `[text][ref]`.
pub(super) fn link(state: &mut State<'_, '_>, silent: bool) -> bool {
    let (old, maximum) = (state.pos, state.pos_max);
    if state.at(state.pos) != Some('[') {
        return false;
    }
    let label_start = state.pos + 1;
    let Some(label_end) = link_label(state, state.pos, true) else {
        return false;
    };
    let target = match inline_target(state, label_end + 1, false) {
        None => return false,
        Some(Ok(target)) => target,
        Some(Err(())) => {
            // markdown-it reads a reference from the place after `)` or the label
            let from = after_inline(state, label_end + 1);
            match reference_target(state, label_start, label_end, from) {
                Some(target) => target,
                None => {
                    state.pos = old;
                    return false;
                }
            }
        }
    };
    if !silent {
        state.pos = label_start;
        state.pos_max = label_end;
        let index = state.push(Kind::LinkOpen, "a", 1);
        if let Some(token) = state.tokens.get_mut(index) {
            token.set_attr("href", Value::Text(target.href));
            if !target.title.is_empty() {
                token.set_attr("title", Value::Text(target.title));
            }
        }
        tokenize(state);
        state.push(Kind::LinkClose, "a", -1);
    }
    state.pos = target.pos;
    state.pos_max = maximum;
    true
}

/// Where markdown-it's link rule stands when an inline target fails and it reads a
/// reference: past the `(...)` it tried (one after where it stopped), or at `pos`.
fn after_inline(state: &State<'_, '_>, pos: usize) -> usize {
    let maximum = state.pos_max;
    if !(pos < maximum && state.at(pos) == Some('(')) {
        return pos;
    }
    let mut at = skip_space(state, pos + 1);
    if let Some(dest) = destination(state.src, at, maximum) {
        if validate_link(&normalize_link(&dest.text)) {
            at = dest.pos;
        }
        let start = at;
        at = skip_space(state, at);
        let read = title(state.src, at, maximum, None);
        if at < maximum && start != at && read.ok {
            at = skip_space(state, read.pos);
        }
    }
    at + 1
}

/// `![alt](src "title")` or `![alt][ref]`.
pub(super) fn image(state: &mut State<'_, '_>, silent: bool) -> bool {
    let (old, maximum) = (state.pos, state.pos_max);
    if state.at(state.pos) != Some('!') {
        return false;
    }
    if state.pos + 1 < maximum && state.at(state.pos + 1) != Some('[') {
        return false;
    }
    let label_start = state.pos + 2;
    let Some(label_end) = link_label(state, state.pos + 1, false) else {
        return false;
    };
    let target = match inline_target(state, label_end + 1, true) {
        None => {
            state.pos = old;
            return false;
        }
        Some(Ok(target)) => target,
        Some(Err(())) => match reference_target(state, label_start, label_end, label_end + 1) {
            Some(target) => target,
            None => {
                state.pos = old;
                return false;
            }
        },
    };
    if !silent {
        let content = state.slice(label_start, label_end);
        let chars: Vec<char> = content.chars().collect();
        let children = super::parse(&chars, state.dialect, state.env);
        let index = state.push(Kind::Image, "img", 0);
        if let Some(token) = state.tokens.get_mut(index) {
            token.set_attr("src", Value::Text(target.href));
            token.set_attr("alt", Value::Text(String::new()));
            token.children = (!children.is_empty()).then_some(children);
            token.content = content;
            if !target.title.is_empty() {
                token.set_attr("title", Value::Text(target.title));
            }
        }
    }
    state.pos = target.pos;
    state.pos_max = maximum;
    true
}

/// `^[text]`: an inline footnote.
pub(super) fn footnote_inline(state: &mut State<'_, '_>, silent: bool) -> bool {
    let (start, maximum) = (state.pos, state.pos_max);
    if start + 2 >= maximum || state.at(start) != Some('^') || state.at(start + 1) != Some('[') {
        return false;
    }
    let label_start = start + 2;
    let Some(label_end) = link_label(state, start + 1, false) else {
        return false;
    };
    if !silent {
        state.env.footnotes_seen = true;
        let id = state.env.footnotes.len();
        let content = state.slice(label_start, label_end);
        state.env.footnotes.push(crate::env::Footnote::default());
        let chars: Vec<char> = content.chars().collect();
        let tokens = super::parse(&chars, state.dialect, state.env);
        if let Some(note) = state.env.footnotes.get_mut(id) {
            note.inline = Some((content, tokens));
        }
        let index = state.push(Kind::FootnoteRef, "", 0);
        if let Some(token) = state.tokens.get_mut(index) {
            token.meta = Some(Note {
                id: Some(id),
                ..Note::default()
            });
        }
    }
    state.pos = label_end + 1;
    state.pos_max = maximum;
    true
}

/// `[^label]`: a reference to a footnote defined.
pub(super) fn footnote_ref(state: &mut State<'_, '_>, silent: bool) -> bool {
    let (start, maximum) = (state.pos, state.pos_max);
    if start + 3 > maximum || state.env.footnote_refs.is_empty() {
        return false;
    }
    if state.at(start) != Some('[') || state.at(start + 1) != Some('^') {
        return false;
    }
    let mut pos = start + 2;
    while pos < maximum {
        match state.at(pos) {
            Some(' ' | '\n') => return false,
            Some(']') => break,
            _ => pos += 1,
        }
    }
    if pos == start + 2 || pos >= maximum {
        return false;
    }
    pos += 1;
    let label = state.slice(start + 2, pos - 1);
    let key = format!(":{label}");
    let Some(known) = state.env.footnote_refs.get(&key).copied() else {
        return false;
    };
    if !silent {
        let id = match known {
            Some(id) => id,
            None => {
                let id = state.env.footnotes.len();
                state.env.footnotes.push(crate::env::Footnote {
                    label: Some(label.clone()),
                    count: 0,
                    inline: None,
                });
                state.env.footnote_refs.insert(key, Some(id));
                id
            }
        };
        let sub_id = state.env.footnotes.get(id).map_or(0, |n| n.count);
        if let Some(note) = state.env.footnotes.get_mut(id) {
            note.count += 1;
        }
        let index = state.push(Kind::FootnoteRef, "", 0);
        if let Some(token) = state.tokens.get_mut(index) {
            token.meta = Some(Note {
                id: Some(id),
                sub_id: Some(sub_id),
                label: Some(label),
            });
        }
    }
    state.pos = pos;
    state.pos_max = maximum;
    true
}

/// `<scheme:...>` or `<address@domain>`.
pub(super) fn autolink(state: &mut State<'_, '_>, silent: bool) -> bool {
    let start = state.pos;
    if state.at(start) != Some('<') {
        return false;
    }
    let mut pos = start;
    loop {
        pos += 1;
        if pos >= state.pos_max {
            return false;
        }
        match state.at(pos) {
            Some('<') => return false,
            Some('>') => break,
            _ => {}
        }
    }
    let url = state.slice(start + 1, pos);
    let full = if is_url(&url) {
        normalize_link(&url)
    } else if is_email(&url) {
        normalize_link(&format!("mailto:{url}"))
    } else {
        return false;
    };
    if !validate_link(&full) {
        return false;
    }
    if !silent {
        let open = state.push(Kind::LinkOpen, "a", 1);
        if let Some(token) = state.tokens.get_mut(open) {
            token.set_attr("href", Value::Text(full));
            "autolink".clone_into(&mut token.markup);
            "auto".clone_into(&mut token.info);
        }
        let text = state.push(Kind::Text, "", 0);
        if let Some(token) = state.tokens.get_mut(text) {
            token.content = normalize_link_text(&url);
        }
        let close = state.push(Kind::LinkClose, "a", -1);
        if let Some(token) = state.tokens.get_mut(close) {
            "autolink".clone_into(&mut token.markup);
            "auto".clone_into(&mut token.info);
        }
    }
    state.pos += url.chars().count() + 2;
    true
}

/// `^[a-zA-Z][a-zA-Z0-9+.\-]{1,31}:([^<>\x00-\x20]*)$`.
fn is_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && (2..=32).contains(&scheme.chars().count())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
        && rest.chars().all(|c| !matches!(c, '<' | '>') && u32::from(c) > 0x20)
}

/// CommonMark's e-mail autolink.
fn is_email(url: &str) -> bool {
    let Some((local, domain)) = url.split_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(c));
    let label_ok = |l: &str| {
        let n = l.chars().count();
        (1..=63).contains(&n)
            && l.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
            && l.chars().last().is_some_and(|c| c.is_ascii_alphanumeric())
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    local_ok && domain.split('.').all(label_ok)
}
