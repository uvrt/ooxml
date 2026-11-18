//! Owns: text with inline emphasis, as docx-agent's `markdown/styled.py` reads
//! `word_set_text`'s text: one paragraph's text in which `**...**` is strong, `*...*`
//! emphasis, and `[text]{Style name}` a span in a named character style.
//!
//! The rules are CommonMark's for `*` (markdown-it's emphasis rule: `5 * 3` and an unclosed
//! `**` stay as they are), with these differences, chosen for prose:
//! - `_` is never emphasis: `file_name_here` and `_draft_` keep their underscores;
//! - nothing else is Markdown: no links, code spans, HTML, entities or line-start blocks;
//! - a backslash before ASCII punctuation writes it literally (`\*` is `*`); before
//!   anything else it is a backslash (`C:\Users`);
//! - a span is `[` text `]` right followed by `{` a name `}` (`{style="Name"}` or Pandoc's
//!   `{custom-style="Name"}` read as the name); its text may hold `**`/`*`. Spans do not
//!   nest.
//!
//! Must not: know which styles a document has (the caller checks the names).

use crate::chars::is_py_space;
use crate::inline::Dialect;
use crate::token::Kind;

/// Private-use stand-ins while the text is parsed: a span's start and end, an underscore.
const OPEN: char = '\u{e000}';
const CLOSE: char = '\u{e001}';
const UNDERSCORE: char = '\u{e002}';

/// A stretch of the plain text, `[start, end)` in characters, with its marks and the
/// character style a span names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    /// Its first character's place in the plain text.
    pub start: usize,
    /// One past its last.
    pub end: usize,
    /// Its text.
    pub text: String,
    /// Inside `**`.
    pub strong: bool,
    /// Inside `*`.
    pub em: bool,
    /// The style a span names.
    pub style: Option<String>,
}

/// A text read: the characters to write, and the pieces that cover them, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Styled {
    /// The characters to write.
    pub plain: String,
    /// The plain text in pieces.
    pub pieces: Vec<Piece>,
}

impl Styled {
    /// Whether any piece is strong, emphasis or in a named style.
    #[must_use]
    pub fn marked(&self) -> bool {
        self.pieces.iter().any(|p| p.strong || p.em || p.style.is_some())
    }

    /// The styles the spans name, each once, in order.
    #[must_use]
    pub fn styles_named(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for style in self.pieces.iter().filter_map(|p| p.style.as_deref()) {
            if !out.contains(&style) {
                out.push(style);
            }
        }
        out
    }
}

/// The plain text and pieces of one paragraph's `text` (no line ends of its own; every
/// other character passes through).
#[must_use]
pub fn parse(text: &str) -> Styled {
    let (prepared, names) = spans(text);
    if names.is_empty() && !prepared.contains(['*', '\\']) {
        let length = text.chars().count();
        let pieces = if text.is_empty() {
            Vec::new()
        } else {
            vec![piece(0, text.to_owned(), (false, false), None)]
        };
        debug_assert!(pieces.iter().all(|p| p.end == length));
        return Styled {
            plain: text.to_owned(),
            pieces,
        };
    }
    let mut reader = Reader {
        pieces: Vec::new(),
        marks: Vec::new(),
        style: None,
        position: 0,
        names: names.into_iter(),
    };
    for token in crate::parse_inline(&prepared, Dialect::Emphasis) {
        match token.kind {
            Kind::StrongOpen => reader.marks.push(Kind::StrongOpen),
            Kind::EmOpen => reader.marks.push(Kind::EmOpen),
            Kind::StrongClose => reader.close(Kind::StrongOpen),
            Kind::EmClose => reader.close(Kind::EmOpen),
            _ => reader.text(&token.content),
        }
    }
    let plain = reader.pieces.iter().map(|p| p.text.as_str()).collect();
    Styled {
        plain,
        pieces: reader.pieces,
    }
}

fn piece(start: usize, text: String, (strong, em): (bool, bool), style: Option<String>) -> Piece {
    Piece {
        start,
        end: start + text.chars().count(),
        text,
        strong,
        em,
        style,
    }
}

/// The pieces as the tokens give them.
struct Reader {
    pieces: Vec<Piece>,
    marks: Vec<Kind>,
    style: Option<String>,
    position: usize,
    names: std::vec::IntoIter<String>,
}

impl Reader {
    /// The last open mark of this kind closes.
    fn close(&mut self, open: Kind) {
        if let Some(at) = self.marks.iter().rposition(|m| *m == open) {
            self.marks.remove(at);
        }
    }

    /// A text token's content: stand-ins open and close spans between its pieces.
    fn text(&mut self, content: &str) {
        let mut chunk = String::new();
        for ch in content.chars() {
            match ch {
                OPEN | CLOSE => {
                    self.add(&std::mem::take(&mut chunk));
                    self.style = if ch == OPEN { self.names.next() } else { None };
                }
                UNDERSCORE => chunk.push('_'),
                other => chunk.push(other),
            }
        }
        self.add(&chunk);
    }

    fn add(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        let marks = (
            self.marks.contains(&Kind::StrongOpen),
            self.marks.contains(&Kind::EmOpen),
        );
        let length = chunk.chars().count();
        match self.pieces.last_mut() {
            Some(last) if (last.strong, last.em) == marks && last.style == self.style => {
                last.text.push_str(chunk);
                last.end += length;
            }
            _ => {
                let made = piece(self.position, chunk.to_owned(), marks, self.style.clone());
                self.pieces.push(made);
            }
        }
        self.position += length;
    }
}

/// `text` with each span's `[`, `]{name}` replaced by stand-ins, and the names in order;
/// escapes kept for the parser; underscores as stand-ins.
fn spans(text: &str) -> (String, Vec<String>) {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut names = Vec::new();
    let mut open_at: Option<usize> = None;
    let mut k = 0;
    while let Some(&ch) = chars.get(k) {
        let next = chars.get(k + 1).copied();
        if ch == '\\' && next.is_some_and(|n| n.is_ascii_punctuation()) {
            out.push(ch);
            out.extend(next);
            k += 2;
            continue;
        }
        match ch {
            '_' => out.push(UNDERSCORE),
            '[' => {
                open_at = Some(out.len());
                out.push(ch);
            }
            ']' if open_at.is_some() && next == Some('{') => {
                if let Some((name, close)) = span_name(&chars, k + 2) {
                    if let Some(slot) = open_at.and_then(|at| out.get_mut(at)) {
                        *slot = OPEN;
                    }
                    out.push(CLOSE);
                    names.push(name);
                    open_at = None;
                    k = close + 1;
                    continue;
                }
                out.push(ch);
            }
            _ => out.push(ch),
        }
        k += 1;
    }
    (out.into_iter().collect(), names)
}

/// The name a span's `{...}` from `from` gives, and the place of its `}`; `None` when it
/// names nothing.
fn span_name(chars: &[char], from: usize) -> Option<(String, usize)> {
    let close = from + chars.get(from..)?.iter().position(|c| *c == '}')?;
    let body = chars.get(from..close)?;
    if body.iter().any(|c| *c == '{' || *c == '\n') {
        return None;
    }
    let name = named(body)?;
    let name: String = name.trim_matches(is_py_space).to_owned();
    (!name.is_empty()).then_some((name, close))
}

/// The name in a span's braces, as `\s*(?:(?:custom-)?style\s*=\s*)?(?:"([^"]+)"|
/// '([^']+)'|([^"'{}]+?))\s*` reads it whole: a quoted name, else a bare one.
fn named(body: &[char]) -> Option<String> {
    let rest = skip_space(body);
    let prefix = ["custom-style", "style"].into_iter().find_map(|word| {
        let word: Vec<char> = word.chars().collect();
        let after = rest.strip_prefix(word.as_slice())?;
        skip_space(after).strip_prefix(&['='][..]).map(skip_space)
    });
    prefix.and_then(value).or_else(|| value(rest))
}

fn skip_space(chars: &[char]) -> &[char] {
    let at = chars.iter().position(|c| !is_py_space(*c)).unwrap_or(chars.len());
    chars.get(at..).unwrap_or_default()
}

/// A quoted value with only white space after it, or a bare one without quotes or braces.
fn value(chars: &[char]) -> Option<String> {
    for quote in ['"', '\''] {
        if let Some(inner) = chars.strip_prefix(&[quote][..]) {
            if let Some(end) = inner.iter().position(|c| *c == quote) {
                let rest = inner.get(end + 1..).unwrap_or_default();
                if end > 0 && rest.iter().all(|c| is_py_space(*c)) {
                    return Some(inner.get(..end).unwrap_or_default().iter().collect());
                }
            }
        }
    }
    let bare = !chars.is_empty() && !chars.iter().any(|c| matches!(c, '"' | '\'' | '{' | '}'));
    bare.then(|| chars.iter().collect())
}
