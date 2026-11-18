//! Owns: the definition rules: a link reference definition (`rules_block/reference.py`:
//! `[label]: destination "title"`, over lines while nothing else starts) kept in the
//! [`crate::env::Env`]; a footnote definition (markdown-it-footnote's `footnote_def`:
//! `[^label]:` and the blocks indented under it, read between two marker tokens the core
//! moves to the end).
//!
//! Must not: read inline content.

use super::{terminates, tokenize, Chain, Parent, State};
use crate::chars::is_str_space;
use crate::env::Reference;
use crate::link::{destination, normalize_reference, title};
use crate::token::{Kind, Note, Token};
use crate::url::{normalize_link, validate_link};

/// The next line of a definition, if it continues it (`getNextLine`).
fn next_line(state: &mut State<'_, '_>, line: usize) -> Option<Vec<char>> {
    let end = state.line_max;
    if line >= end || state.is_empty(line) {
        return None;
    }
    let continuation = state.is_code_block(line) || state.s(line) < 0;
    if !continuation {
        let old = state.parent;
        state.parent = Parent::Reference;
        let stop = terminates(state, Chain::Reference, line, end);
        state.parent = old;
        if stop {
            return None;
        }
    }
    Some(
        state
            .src
            .get(state.first(line)..(state.e(line) + 1).min(state.src.len()))
            .unwrap_or_default()
            .to_vec(),
    )
}

/// The text read so far, the line after it, and how to take the next line.
struct Reader {
    text: Vec<char>,
    next: usize,
}

impl Reader {
    fn more(&mut self, state: &mut State<'_, '_>) -> bool {
        match next_line(state, self.next) {
            Some(line) => {
                self.text.extend(line);
                self.next += 1;
                true
            }
            None => false,
        }
    }

    fn at(&self, k: usize) -> Option<char> {
        self.text.get(k).copied()
    }

    /// The label's closing `]`, lines taken while it is open; `None` for a nested `[`.
    fn label_end(&mut self, state: &mut State<'_, '_>) -> Option<usize> {
        let mut pos = 1;
        while pos < self.text.len() {
            match self.at(pos) {
                Some('[') => return None,
                Some(']') => return Some(pos),
                Some('\n') => {
                    self.more(state);
                }
                Some('\\') => {
                    pos += 1;
                    if pos < self.text.len() && self.at(pos) == Some('\n') {
                        self.more(state);
                    }
                }
                _ => {}
            }
            pos += 1;
        }
        None
    }

    /// Skip spaces and line ends (taking the next line at each) from `pos`.
    fn skip_space(&mut self, state: &mut State<'_, '_>, mut pos: usize) -> usize {
        while pos < self.text.len() {
            match self.at(pos) {
                Some('\n') => {
                    self.more(state);
                }
                Some(c) if is_str_space(c) => {}
                _ => break,
            }
            pos += 1;
        }
        pos
    }
}

pub(super) fn reference(state: &mut State<'_, '_>, start: usize, _end: usize, silent: bool) -> bool {
    let pos = state.first(start);
    if state.is_code_block(start) || state.at(pos) != Some('[') {
        return false;
    }
    let first = state
        .src
        .get(pos..(state.e(start) + 1).min(state.src.len()))
        .unwrap_or_default()
        .to_vec();
    let mut reader = Reader {
        text: first,
        next: start + 1,
    };
    let label_end = reader.label_end(state);
    let Some(label_end) = label_end.filter(|e| reader.at(e + 1) == Some(':')) else {
        return false;
    };
    let mut pos = reader.skip_space(state, label_end + 2);
    let Some(dest) = destination(&reader.text, pos, reader.text.len()) else {
        return false;
    };
    let href = normalize_link(&dest.text);
    if !validate_link(&href) {
        return false;
    }
    pos = dest.pos;
    let (dest_end, dest_line) = (pos, reader.next);
    let title_start = pos;
    pos = reader.skip_space(state, pos);
    let mut found = title(&reader.text, pos, reader.text.len(), None);
    while found.can_continue {
        let old = reader.text.len();
        if !reader.more(state) {
            break;
        }
        pos = old;
        found = title(&reader.text, pos, reader.text.len(), Some(&found));
    }
    let maximum = reader.text.len();
    let mut text_title;
    if pos < maximum && title_start != pos && found.ok {
        text_title = found.text.clone();
        pos = found.pos;
    } else {
        text_title = String::new();
        pos = dest_end;
        reader.next = dest_line;
    }
    let skip = |reader: &Reader, mut pos: usize| {
        while pos < maximum && reader.at(pos).is_some_and(is_str_space) {
            pos += 1;
        }
        pos
    };
    pos = skip(&reader, pos);
    if pos < maximum && reader.at(pos) != Some('\n') && !text_title.is_empty() {
        text_title.clear();
        pos = skip(&reader, dest_end);
        reader.next = dest_line;
    }
    if pos < maximum && reader.at(pos) != Some('\n') {
        return false;
    }
    let label = normalize_reference(
        &reader
            .text
            .get(1..label_end)
            .unwrap_or_default()
            .iter()
            .collect::<String>(),
    );
    if label.is_empty() {
        return false;
    }
    if silent {
        return true;
    }
    state.line = reader.next;
    state.env.references.entry(label).or_insert(Reference {
        href,
        title: text_title,
    });
    true
}

pub(super) fn footnote_def(state: &mut State<'_, '_>, start: usize, end: usize, silent: bool) -> bool {
    if state.is_code_block(start) {
        return false;
    }
    let begin = state.first(start);
    let maximum = state.e(start);
    if begin + 4 > maximum || state.at(begin) != Some('[') || state.at(begin + 1) != Some('^') {
        return false;
    }
    let mut pos = begin + 2;
    while pos < maximum {
        match state.at(pos) {
            Some(' ') => return false,
            Some(']') => break,
            _ => pos += 1,
        }
    }
    if pos == begin + 2 {
        return false;
    }
    pos += 1;
    if pos >= maximum || state.at(pos) != Some(':') {
        return false;
    }
    if silent {
        return true;
    }
    pos += 1;
    let label = state.slice(begin + 2, pos - 2);
    state.env.footnotes_seen = true;
    state.env.footnote_refs.insert(format!(":{label}"), None);
    let mut open = Token::new(Kind::FootnoteReferenceOpen, "", 1);
    open.meta = Some(Note {
        label: Some(label),
        ..Note::default()
    });
    open.level = state.level;
    state.level += 1;
    state.tokens.push(open);
    let open_index = state.tokens.len() - 1;
    let (old_b, old_t, old_s, old_parent) = (state.b(start), state.t(start), state.s(start), state.parent);
    let after_colon = pos;
    let initial = state.s(start) + isize::try_from(pos).unwrap_or(0) - isize::try_from(state.first(start)).unwrap_or(0);
    let mut offset = initial;
    while pos < maximum {
        match state.at(pos) {
            Some('\t') => offset += 4 - offset % 4,
            Some(' ') => offset += 1,
            _ => break,
        }
        pos += 1;
    }
    state.set_t(start, pos - after_colon);
    state.set_s(start, offset - initial);
    state.set_b(start, after_colon);
    state.blk_indent += 4;
    state.parent = Parent::Footnote;
    if state.s(start) < state.blk_indent {
        state.set_s(start, state.s(start) + state.blk_indent);
    }
    tokenize(state, start, end);
    state.parent = old_parent;
    state.blk_indent -= 4;
    state.set_t(start, old_t);
    state.set_s(start, old_s);
    state.set_b(start, old_b);
    let line = state.line;
    if let Some(token) = state.token(open_index) {
        token.map = Some((start, line));
    }
    let mut close = Token::new(Kind::FootnoteReferenceClose, "", -1);
    state.level -= 1;
    close.level = state.level;
    state.tokens.push(close);
    true
}
