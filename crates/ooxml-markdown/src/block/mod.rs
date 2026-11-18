//! Owns: markdown-it's block parser (`parser_block.py`, `rules_block/state_block.py`):
//! the source cut into lines with their indents, the state blocks are read in (the
//! required indent, the parent kind, tightness), the rule chain run at each line, and the
//! chains of rules that may end a paragraph, a quote, a list or a reference.
//!
//! Must not: parse inline content (a block's inline token keeps its text; the core runs
//! the inline parser over it).

mod blockquote;
mod fence;
mod html;
mod leaf;
mod list;
mod reference;
mod table;

use crate::chars::is_str_space;
use crate::env::Env;
use crate::token::{Kind, Token};

/// markdown-it's recursion limit (`maxNesting` of the `commonmark` preset).
const MAX_NESTING: i32 = 20;

/// A chain of rules that may end a block (markdown-it's `alt` names).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chain {
    Paragraph,
    Reference,
    Blockquote,
    List,
}

/// The block a rule is read inside (markdown-it's `parentType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Parent {
    Root,
    Paragraph,
    Blockquote,
    List,
    Reference,
    Footnote,
    Table,
}

/// The block parser's state (markdown-it's `StateBlock`).
pub(crate) struct State<'s, 'e> {
    pub(crate) src: &'s [char],
    pub(crate) env: &'e mut Env,
    pub(crate) tokens: Vec<Token>,
    /// Each line's start, end, first non-space (characters), indent (tabs expanded) and
    /// the indent of what a container took off it; one more, empty, past the last.
    pub(crate) b_marks: Vec<usize>,
    pub(crate) e_marks: Vec<usize>,
    pub(crate) t_shift: Vec<usize>,
    pub(crate) s_count: Vec<isize>,
    pub(crate) bs_count: Vec<isize>,
    pub(crate) blk_indent: isize,
    pub(crate) line: usize,
    pub(crate) line_max: usize,
    pub(crate) tight: bool,
    pub(crate) list_indent: isize,
    pub(crate) parent: Parent,
    pub(crate) level: i32,
    /// Whether HTML is read (`html` of the preset).
    pub(crate) html: bool,
}

impl<'s, 'e> State<'s, 'e> {
    /// The state over `src`, its lines measured.
    pub(crate) fn new(src: &'s [char], env: &'e mut Env, html: bool) -> State<'s, 'e> {
        let mut state = State {
            src,
            env,
            tokens: Vec::new(),
            b_marks: Vec::new(),
            e_marks: Vec::new(),
            t_shift: Vec::new(),
            s_count: Vec::new(),
            bs_count: Vec::new(),
            blk_indent: 0,
            line: 0,
            line_max: 0,
            tight: false,
            list_indent: -1,
            parent: Parent::Root,
            level: 0,
            html,
        };
        let length = src.len();
        let mut start = 0;
        while start < length {
            let end = src
                .get(start..)
                .and_then(|r| r.iter().position(|c| *c == '\n'))
                .map_or(length, |k| start + k);
            let (mut indent, mut offset, mut pos) = (0usize, 0isize, start);
            while pos < end {
                let Some(ch) = state.at(pos).filter(|c| is_str_space(*c)) else {
                    break;
                };
                indent += 1;
                offset += if ch == '\t' { 4 - offset % 4 } else { 1 };
                pos += 1;
            }
            if pos == end && end == length {
                break;
            }
            state.push_line(start, end, indent, offset);
            start = if end < length { end + 1 } else { length };
        }
        state.push_line(length, length, 0, 0);
        state.line_max = state.b_marks.len() - 1;
        state
    }

    fn push_line(&mut self, start: usize, end: usize, indent: usize, offset: isize) {
        self.b_marks.push(start);
        self.e_marks.push(end);
        self.t_shift.push(indent);
        self.s_count.push(offset);
        self.bs_count.push(0);
    }

    pub(crate) fn at(&self, pos: usize) -> Option<char> {
        self.src.get(pos).copied()
    }

    pub(crate) fn slice(&self, start: usize, end: usize) -> String {
        self.src.get(start..end.max(start)).unwrap_or_default().iter().collect()
    }

    pub(crate) fn b(&self, line: usize) -> usize {
        self.b_marks.get(line).copied().unwrap_or(self.src.len())
    }

    pub(crate) fn e(&self, line: usize) -> usize {
        self.e_marks.get(line).copied().unwrap_or(self.src.len())
    }

    pub(crate) fn t(&self, line: usize) -> usize {
        self.t_shift.get(line).copied().unwrap_or(0)
    }

    pub(crate) fn s(&self, line: usize) -> isize {
        self.s_count.get(line).copied().unwrap_or(0)
    }

    pub(crate) fn bs(&self, line: usize) -> isize {
        self.bs_count.get(line).copied().unwrap_or(0)
    }

    /// The line's first non-space character's place.
    pub(crate) fn first(&self, line: usize) -> usize {
        self.b(line) + self.t(line)
    }

    pub(crate) fn set_b(&mut self, line: usize, value: usize) {
        if let Some(slot) = self.b_marks.get_mut(line) {
            *slot = value;
        }
    }

    pub(crate) fn set_t(&mut self, line: usize, value: usize) {
        if let Some(slot) = self.t_shift.get_mut(line) {
            *slot = value;
        }
    }

    pub(crate) fn set_s(&mut self, line: usize, value: isize) {
        if let Some(slot) = self.s_count.get_mut(line) {
            *slot = value;
        }
    }

    pub(crate) fn set_bs(&mut self, line: usize, value: isize) {
        if let Some(slot) = self.bs_count.get_mut(line) {
            *slot = value;
        }
    }

    /// Push a block token; its index.
    pub(crate) fn push(&mut self, kind: Kind, tag: &str, nesting: i8) -> usize {
        let mut token = Token::new(kind, tag, nesting);
        token.block = true;
        if nesting < 0 {
            self.level -= 1;
        }
        token.level = self.level;
        if nesting > 0 {
            self.level += 1;
        }
        self.tokens.push(token);
        self.tokens.len() - 1
    }

    /// The token at `index`, to fill in.
    pub(crate) fn token(&mut self, index: usize) -> Option<&mut Token> {
        self.tokens.get_mut(index)
    }

    pub(crate) fn is_empty(&self, line: usize) -> bool {
        self.first(line) >= self.e(line)
    }

    pub(crate) fn skip_empty_lines(&self, mut from: usize) -> usize {
        while from < self.line_max && self.is_empty(from) {
            from += 1;
        }
        from
    }

    pub(crate) fn skip_spaces(&self, mut pos: usize) -> usize {
        while self.at(pos).is_some_and(is_str_space) {
            pos += 1;
        }
        pos
    }

    pub(crate) fn skip_spaces_back(&self, mut pos: usize, minimum: usize) -> usize {
        while pos > minimum {
            pos -= 1;
            if !self.at(pos).is_some_and(is_str_space) {
                return pos + 1;
            }
        }
        pos
    }

    pub(crate) fn skip_chars(&self, mut pos: usize, ch: char) -> usize {
        while self.at(pos) == Some(ch) {
            pos += 1;
        }
        pos
    }

    pub(crate) fn skip_chars_back(&self, mut pos: usize, ch: char, minimum: usize) -> usize {
        while pos > minimum {
            pos -= 1;
            if self.at(pos) != Some(ch) {
                return pos + 1;
            }
        }
        pos
    }

    /// The lines `[begin, end)` without `indent` columns of their indent (markdown-it's
    /// `getLines`).
    pub(crate) fn get_lines(&self, begin: usize, end: usize, indent: isize, keep_last_lf: bool) -> String {
        let mut out = String::new();
        for line in begin..end {
            let mut line_indent: isize = 0;
            let start = self.b(line);
            let mut first = start;
            let last = if line + 1 < end || keep_last_lf {
                self.e(line) + 1
            } else {
                self.e(line)
            };
            while first < last && line_indent < indent {
                let Some(ch) = self.at(first) else { break };
                if is_str_space(ch) {
                    line_indent += if ch == '\t' {
                        4 - (line_indent + self.bs(line)) % 4
                    } else {
                        1
                    };
                } else if first - start < self.t(line) {
                    line_indent += 1;
                } else {
                    break;
                }
                first += 1;
            }
            if line_indent > indent {
                out.extend(std::iter::repeat_n(
                    ' ',
                    usize::try_from(line_indent - indent).unwrap_or(0),
                ));
            }
            out.push_str(&self.slice(first, last.min(self.src.len())));
        }
        out
    }

    /// Whether the line is an indented code block's (4 columns past the block's indent).
    pub(crate) fn is_code_block(&self, line: usize) -> bool {
        self.s(line) - self.blk_indent >= 4
    }
}

/// A block rule: whether it read a block at `start` (and moved `line` past it).
pub(crate) type Rule = fn(&mut State<'_, '_>, usize, usize, bool) -> bool;

/// The rules in order, each with the chains it may end (markdown-it's ruler with the
/// footnote plugin's `footnote_def` before `reference`).
const RULES: &[(Rule, &[Chain])] = &[
    (table::table, &[Chain::Paragraph, Chain::Reference]),
    (leaf::code, &[]),
    (
        fence::fence,
        &[Chain::Paragraph, Chain::Reference, Chain::Blockquote, Chain::List],
    ),
    (
        blockquote::blockquote,
        &[Chain::Paragraph, Chain::Reference, Chain::Blockquote, Chain::List],
    ),
    (
        leaf::hr,
        &[Chain::Paragraph, Chain::Reference, Chain::Blockquote, Chain::List],
    ),
    (list::list, &[Chain::Paragraph, Chain::Reference, Chain::Blockquote]),
    (reference::footnote_def, &[Chain::Paragraph, Chain::Reference]),
    (reference::reference, &[]),
    (
        html::html_block,
        &[Chain::Paragraph, Chain::Reference, Chain::Blockquote],
    ),
    (leaf::heading, &[Chain::Paragraph, Chain::Reference, Chain::Blockquote]),
    (leaf::lheading, &[]),
    (leaf::paragraph, &[]),
];

/// Whether a rule of `chain` starts a block at `line` (looking only).
pub(crate) fn terminates(state: &mut State<'_, '_>, chain: Chain, line: usize, end: usize) -> bool {
    RULES
        .iter()
        .filter(|(_, alt)| alt.contains(&chain))
        .any(|(rule, _)| rule(state, line, end, true))
}

/// Read the blocks of lines `[start, end)` (markdown-it's `ParserBlock.tokenize`).
pub(crate) fn tokenize(state: &mut State<'_, '_>, start: usize, end: usize) {
    let mut line = start;
    let mut has_empty_lines = false;
    while line < end {
        line = state.skip_empty_lines(line);
        state.line = line;
        if line >= end || state.s(line) < state.blk_indent {
            break;
        }
        if state.level >= MAX_NESTING {
            state.line = end;
            break;
        }
        let ok = RULES.iter().any(|(rule, _)| rule(state, line, end, false));
        if !ok || state.line <= line {
            // the paragraph rule reads any line: a rule that read nothing ends the read
            state.line = end;
            break;
        }
        state.tight = !has_empty_lines;
        line = state.line;
        if line > 0 && line - 1 < end && state.is_empty(line - 1) {
            has_empty_lines = true;
        }
        if line < end && state.is_empty(line) {
            has_empty_lines = true;
            line += 1;
            state.line = line;
        }
    }
}
