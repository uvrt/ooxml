//! Owns: the HTML block rule (`rules_block/html_block.py`): the seven kinds CommonMark
//! names, each by how it starts and what ends it.
//!
//! Must not: read HTML beyond the starts and ends.

use super::State;
use crate::chars::is_py_space;
use crate::html::{close_tag, open_tag, BLOCK_NAMES};
use crate::token::Kind;

/// How an HTML block ends.
#[derive(Clone, Copy)]
enum End {
    /// A line holding this text, ignoring case for a closing tag of the raw kinds.
    Raw,
    Text(&'static str),
    /// An empty line.
    Blank,
}

fn starts_with_ci(line: &[char], word: &str) -> bool {
    let n = word.chars().count();
    line.get(..n)
        .is_some_and(|head| head.iter().zip(word.chars()).all(|(a, b)| a.to_ascii_lowercase() == b))
}

/// After a tag name at `k`: white space, `>` or the end (`/>` too where `slash`).
fn name_ends(line: &[char], k: usize, slash: bool) -> bool {
    match line.get(k) {
        None => true,
        Some(c) if is_py_space(*c) || *c == '>' => true,
        Some('/') => slash && line.get(k + 1) == Some(&'>'),
        _ => false,
    }
}

/// The kind of HTML block `line` starts, and whether it may end a paragraph.
fn start(line: &[char]) -> Option<(End, bool)> {
    let raw = ["script", "pre", "style", "textarea"];
    let tail = line.get(1..)?;
    if raw
        .iter()
        .any(|w| starts_with_ci(tail, w) && name_ends(line, 1 + w.len(), false))
    {
        return Some((End::Raw, true));
    }
    let text = |s: &str| line.iter().copied().take(s.chars().count()).eq(s.chars());
    if text("<!--") {
        return Some((End::Text("-->"), true));
    }
    if text("<?") {
        return Some((End::Text("?>"), true));
    }
    if text("<!") && line.get(2).is_some_and(char::is_ascii_alphabetic) {
        return Some((End::Text(">"), true));
    }
    if text("<![CDATA[") {
        return Some((End::Text("]]>"), true));
    }
    let after = if line.get(1) == Some(&'/') { 2 } else { 1 };
    let named = BLOCK_NAMES
        .iter()
        .any(|w| line.get(after..).is_some_and(|r| starts_with_ci(r, w)) && name_ends(line, after + w.len(), true));
    if named {
        return Some((End::Blank, true));
    }
    let tag_end = open_tag(line, 0).or_else(|| close_tag(line, 0))?;
    line.get(tag_end..)?
        .iter()
        .all(|c| is_py_space(*c))
        .then_some((End::Blank, false))
}

fn ends(line: &[char], end: End) -> bool {
    match end {
        End::Blank => line.is_empty(),
        End::Text(t) => {
            let t: Vec<char> = t.chars().collect();
            line.windows(t.len()).any(|w| w == t.as_slice())
        }
        End::Raw => ["script", "pre", "style", "textarea"].iter().any(|w| {
            let n = w.len() + 3;
            line.windows(n).any(|win| {
                win.first() == Some(&'<')
                    && win.get(1) == Some(&'/')
                    && starts_with_ci(win.get(2..).unwrap_or_default(), w)
                    && win.last() == Some(&'>')
            })
        }),
    }
}

pub(super) fn html_block(state: &mut State<'_, '_>, start_line: usize, end_line: usize, silent: bool) -> bool {
    let pos = state.first(start_line);
    let maximum = state.e(start_line);
    if state.is_code_block(start_line) || !state.html || state.at(pos) != Some('<') {
        return false;
    }
    let line: Vec<char> = state.src.get(pos..maximum).unwrap_or_default().to_vec();
    let Some((end, interrupts)) = start(&line) else {
        return false;
    };
    if silent {
        return interrupts;
    }
    let mut next = start_line + 1;
    if !ends(&line, end) {
        let blank = matches!(end, End::Blank);
        while next < end_line {
            if state.s(next) < state.blk_indent && (blank || !state.is_empty(next)) {
                break;
            }
            let text: Vec<char> = state
                .src
                .get(state.first(next)..state.e(next))
                .unwrap_or_default()
                .to_vec();
            if ends(&text, end) {
                if !text.is_empty() {
                    next += 1;
                }
                break;
            }
            next += 1;
        }
    }
    state.line = next;
    let content = state.get_lines(start_line, next, state.blk_indent, true);
    let index = state.push(Kind::HtmlBlock, "", 0);
    if let Some(token) = state.token(index) {
        token.map = Some((start_line, next));
        token.content = content;
    }
    true
}
