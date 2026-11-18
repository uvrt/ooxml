//! Owns: markdown-it's inline parser (`parser_inline.py`, `rules_inline/state_inline.py`):
//! the state a paragraph's text is read in -- the source, the pending text, the tokens and
//! the delimiter lists of each nesting level -- the rule chain of a [`Dialect`] run at each
//! position, and the post-processing chain (pairs balanced, emphasis made, text joined).
//!
//! Must not: read anything but the text it is given.

mod links;
mod pairs;
mod rules;
mod spans;

use crate::env::Env;
use crate::token::{Kind, Token};
use std::collections::HashMap;

/// The rules a text is read with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// markdown-it's `zero` preset with `emphasis` and `escape` enabled: `*` and `_`
    /// emphasis and backslash escapes, nothing else (docx-agent's styled text).
    Emphasis,
    /// markdown-it's `commonmark` preset with GFM tables and strikethrough and the
    /// footnote plugin (docx-agent's `insert_markdown`).
    Document,
}

/// markdown-it's recursion limit (`maxNesting` of the `zero` and `commonmark` presets).
const MAX_NESTING: i32 = 20;

/// An emphasis-like marker run (markdown-it's `Delimiter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Delimiter {
    /// The marker character.
    pub(crate) marker: char,
    /// The run's length.
    pub(crate) length: usize,
    /// The marker's token.
    pub(crate) token: usize,
    /// The delimiter that closes it, once paired.
    pub(crate) end: Option<usize>,
    /// Whether it can open.
    pub(crate) open: bool,
    /// Whether it can close.
    pub(crate) close: bool,
}

/// The inline parser's state (markdown-it's `StateInline`).
pub(crate) struct State<'s, 'e> {
    pub(crate) src: &'s [char],
    pub(crate) env: &'e mut Env,
    pub(crate) dialect: Dialect,
    /// Where `skip_token` ended from each place.
    cache: HashMap<usize, usize>,
    /// Each backtick run length's last place (`backticks`), once scanned.
    pub(crate) backticks: Option<HashMap<usize, usize>>,
    /// The last place of each HTML terminator, once looked for.
    pub(crate) html_terminators: HashMap<&'static str, Option<usize>>,
    pub(crate) pos: usize,
    pub(crate) pos_max: usize,
    pub(crate) level: i32,
    pending: String,
    pending_level: i32,
    pub(crate) tokens: Vec<Token>,
    /// Every delimiter list, the top level's first; `current` is the open level's.
    pub(crate) lists: Vec<Vec<Delimiter>>,
    pub(crate) current: usize,
    previous: Vec<usize>,
}

impl<'s, 'e> State<'s, 'e> {
    fn new(src: &'s [char], env: &'e mut Env, dialect: Dialect) -> State<'s, 'e> {
        State {
            src,
            env,
            dialect,
            cache: HashMap::new(),
            backticks: None,
            html_terminators: HashMap::new(),
            pos: 0,
            pos_max: src.len(),
            level: 0,
            pending: String::new(),
            pending_level: 0,
            tokens: Vec::new(),
            lists: vec![Vec::new()],
            current: 0,
            previous: Vec::new(),
        }
    }

    /// The character at `at`.
    pub(crate) fn at(&self, at: usize) -> Option<char> {
        self.src.get(at).copied()
    }

    /// The source from `start` to `end`.
    pub(crate) fn slice(&self, start: usize, end: usize) -> String {
        self.src.get(start..end).unwrap_or_default().iter().collect()
    }

    /// Text not yet a token.
    pub(crate) fn append_pending(&mut self, text: &str) {
        self.pending.push_str(text);
    }

    /// The pending text (the newline rule trims its spaces).
    pub(crate) fn pending_mut(&mut self) -> &mut String {
        &mut self.pending
    }

    /// The last place `term` occurs in the source, or `None`.
    pub(crate) fn last_of(&mut self, term: &'static str) -> Option<usize> {
        let src = self.src;
        *self.html_terminators.entry(term).or_insert_with(|| {
            let needle: Vec<char> = term.chars().collect();
            src.windows(needle.len()).rposition(|w| w == needle.as_slice())
        })
    }

    fn push_pending(&mut self) {
        let mut token = Token::new(Kind::Text, "", 0);
        token.content = std::mem::take(&mut self.pending);
        token.level = self.pending_level;
        self.tokens.push(token);
    }

    /// Push a token (the pending text first, as a text token); its index.
    pub(crate) fn push(&mut self, kind: Kind, tag: &str, nesting: i8) -> usize {
        if !self.pending.is_empty() {
            self.push_pending();
        }
        let mut token = Token::new(kind, tag, nesting);
        if nesting < 0 {
            self.level -= 1;
            self.current = self.previous.pop().unwrap_or(0);
        }
        token.level = self.level;
        if nesting > 0 {
            self.level += 1;
            self.previous.push(self.current);
            self.lists.push(Vec::new());
            self.current = self.lists.len() - 1;
        }
        self.pending_level = self.level;
        self.tokens.push(token);
        self.tokens.len() - 1
    }

    /// The open level's delimiters.
    pub(crate) fn delimiters(&mut self) -> Option<&mut Vec<Delimiter>> {
        self.lists.get_mut(self.current)
    }
}

/// A rule: whether it read something at the state's position (and moved past it).
type Rule = fn(&mut State<'_, '_>, bool) -> bool;

fn rules(dialect: Dialect) -> &'static [Rule] {
    match dialect {
        Dialect::Emphasis => &[rules::text, rules::escape, rules::emphasis],
        Dialect::Document => &[
            rules::text,
            rules::newline,
            rules::escape,
            spans::backticks,
            spans::strikethrough,
            rules::emphasis,
            links::link,
            links::image,
            links::footnote_inline,
            links::footnote_ref,
            links::autolink,
            spans::html_inline,
            spans::entity,
        ],
    }
}

/// The inline tokens of `src` (markdown-it's `ParserInline.parse`).
pub(crate) fn parse(src: &[char], dialect: Dialect, env: &mut Env) -> Vec<Token> {
    let mut state = State::new(src, env, dialect);
    tokenize(&mut state);
    pairs::balance_pairs(&mut state);
    pairs::strikethrough(&mut state);
    pairs::emphasis(&mut state);
    pairs::fragments_join(&mut state.tokens);
    state.tokens
}

/// Skip one token at the position, every rule run looking only (`skipToken`).
pub(crate) fn skip_token(state: &mut State<'_, '_>) {
    let pos = state.pos;
    if let Some(&cached) = state.cache.get(&pos) {
        state.pos = cached;
        return;
    }
    let mut ok = false;
    if state.level < MAX_NESTING {
        for rule in rules(state.dialect) {
            state.level += 1;
            ok = rule(state, true);
            state.level -= 1;
            if ok {
                break;
            }
        }
    } else {
        state.pos = state.pos_max;
    }
    if !ok || state.pos <= pos {
        state.pos = state.pos.max(pos + 1);
    }
    state.cache.insert(pos, state.pos);
}

/// Read tokens from the position to `pos_max` (`tokenize`).
pub(crate) fn tokenize(state: &mut State<'_, '_>) {
    let rules = rules(state.dialect);
    let end = state.pos_max;
    while state.pos < end {
        let pos = state.pos;
        let mut ok = false;
        if state.level < MAX_NESTING {
            for rule in rules {
                ok = rule(state, false);
                if ok {
                    // a rule that read nothing would loop: its character is text instead
                    ok = state.pos > pos;
                    state.pos = state.pos.max(pos);
                    break;
                }
            }
        }
        if ok {
            if state.pos >= end {
                break;
            }
            continue;
        }
        if let Some(ch) = state.at(state.pos) {
            state.pending.push(ch);
        }
        state.pos += 1;
    }
    if !state.pending.is_empty() {
        state.push_pending();
    }
}
