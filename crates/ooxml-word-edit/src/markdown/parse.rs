//! Owns: Markdown read into the neutral model insert_markdown writes (docx-agent's
//! `markdown/parse.py`, `parse_model`): [`ooxml_markdown::parse`]'s tokens as blocks --
//! footnote definitions last, in the order of their first reference; a soft line break a
//! space in its text; an HTML block that is only comments a comment per comment, one that
//! is only `<br>` a paragraph of line breaks; a table whose header row is empty a table
//! with none.
//!
//! Must not: write a document.

use crate::{EditError, Result};
use ooxml_markdown::env::Env;
use ooxml_markdown::{Kind, Token, Value};
use ooxml_word_model::markdown::model::{Block, Inline, ListBlock, Mark, Marks, Table};

/// Python's `\s` (`str.isspace`, which also takes the information separators).
pub(crate) fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.strip()`.
pub(crate) fn py_strip(text: &str) -> &str {
    text.trim_matches(py_space)
}

/// `text` as model blocks.
///
/// # Errors
/// An inline footnote (`^[text]`), which has no label to write it by.
pub(crate) fn parse_model(text: &str) -> Result<Vec<Block>> {
    let (tokens, env) = ooxml_markdown::parse(text);
    let mut reader = Reader { tokens: &tokens, env: &env };
    Ok(reader.blocks(0, None)?.0)
}

struct Reader<'a> {
    tokens: &'a [Token],
    env: &'a Env,
}

impl Reader<'_> {
    fn kind(&self, i: usize) -> Option<Kind> {
        self.tokens.get(i).map(|t| t.kind)
    }

    fn children(&self, i: usize) -> &[Token] {
        self.tokens.get(i).and_then(|t| t.children.as_deref()).unwrap_or(&[])
    }

    fn label(&self, token: &Token) -> Result<String> {
        token
            .meta
            .as_ref()
            .and_then(|m| m.id)
            .and_then(|id| self.env.footnotes.get(id))
            .and_then(|f| f.label.clone())
            .ok_or_else(|| EditError::Refused("an inline footnote (^[text]) is not inserted: write [^label] and its definition".to_owned()))
    }

    fn blocks(&mut self, mut i: usize, until: Option<Kind>) -> Result<(Vec<Block>, usize)> {
        let mut out = Vec::new();
        while let Some(token) = self.tokens.get(i) {
            let kind = token.kind;
            if until == Some(kind) {
                return Ok((out, i + 1));
            }
            match kind {
                Kind::HtmlBlock => {
                    out.extend(html_block(&token.content));
                    i += 1;
                }
                Kind::ParagraphOpen => {
                    out.push(Block::Paragraph(self.inline(self.children(i + 1))?));
                    i += 3;
                }
                Kind::HeadingOpen => {
                    let level = token.tag.get(1..).and_then(|l| l.parse().ok()).unwrap_or(1);
                    out.push(Block::Heading(level, self.inline(self.children(i + 1))?));
                    i += 3;
                }
                Kind::Fence | Kind::CodeBlock => {
                    out.push(Block::Code(token.content.clone()));
                    i += 1;
                }
                Kind::BlockquoteOpen => {
                    let (inner, next) = self.blocks(i + 1, Some(Kind::BlockquoteClose))?;
                    out.push(Block::Quote(inner));
                    i = next;
                }
                Kind::BulletListOpen | Kind::OrderedListOpen => {
                    let (list, next) = self.list(i)?;
                    out.push(Block::List(list));
                    i = next;
                }
                Kind::TableOpen => {
                    out.push(self.table(i + 1)?);
                    while self.kind(i).is_some_and(|k| k != Kind::TableClose) {
                        i += 1;
                    }
                    i += 1;
                }
                Kind::Hr => {
                    out.push(Block::Rule);
                    i += 1;
                }
                Kind::FootnoteBlockOpen => {
                    i += 1;
                    while let Some(open) = self.tokens.get(i).filter(|t| t.kind != Kind::FootnoteBlockClose) {
                        let label = self.label(open)?;
                        let (inner, next) = self.blocks(i + 1, Some(Kind::FootnoteClose))?;
                        out.push(Block::Footnote(label, inner));
                        i = next;
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        }
        Ok((out, i))
    }

    fn list(&mut self, mut i: usize) -> Result<(ListBlock, usize)> {
        let open = self.tokens.get(i).ok_or_else(|| EditError::Refused("a list ends early".to_owned()))?;
        let ordered = open.kind == Kind::OrderedListOpen;
        let close = if ordered { Kind::OrderedListClose } else { Kind::BulletListClose };
        let start = match open.attr("start") {
            Some(Value::Number(n)) if ordered => *n,
            Some(Value::Text(t)) if ordered => t.parse().unwrap_or(1),
            _ => 1,
        };
        let alternate = matches!(open.markup.as_str(), "*" | "+" | ")");
        let mut items = Vec::new();
        i += 1;
        while self.kind(i).is_some_and(|k| k != close) {
            let (inner, next) = self.blocks(i + 1, Some(Kind::ListItemClose))?;
            items.push(inner);
            i = next;
        }
        Ok((
            ListBlock {
                ordered,
                start,
                items,
                alternate,
            },
            i + 1,
        ))
    }

    fn table(&mut self, mut i: usize) -> Result<Block> {
        let mut rows: Vec<Vec<Vec<Inline>>> = Vec::new();
        let mut aligns = Vec::new();
        let mut current: Vec<Vec<Inline>> = Vec::new();
        let mut header = true;
        while let Some(token) = self.tokens.get(i).filter(|t| t.kind != Kind::TableClose) {
            match token.kind {
                Kind::TrOpen => current = Vec::new(),
                Kind::TrClose => rows.push(std::mem::take(&mut current)),
                Kind::TbodyOpen => header = false,
                Kind::ThOpen | Kind::TdOpen => {
                    if token.kind == Kind::ThOpen && header {
                        let style = match token.attr("style") {
                            Some(Value::Text(s)) => s.as_str(),
                            _ => "",
                        };
                        aligns.push(align_of(style));
                    }
                    current.push(self.inline(self.children(i + 1))?);
                    i += 2;
                }
                _ => {}
            }
            i += 1;
        }
        let mut rows = rows.into_iter();
        let first = rows.next().unwrap_or_default();
        let rest: Vec<_> = rows.collect();
        if !first.is_empty() && !first.iter().any(|cell| has_text(cell)) {
            // An empty header row (`| | |`): a table with no header row (round 24, W2).
            return Ok(Block::Table(Table {
                header: Vec::new(),
                rows: rest,
                aligns,
                headed: false,
            }));
        }
        Ok(Block::Table(Table {
            header: first,
            rows: rest,
            aligns,
            headed: true,
        }))
    }

    fn inline(&self, children: &[Token]) -> Result<Vec<Inline>> {
        let mut items = Vec::new();
        let mut marks: Vec<Mark> = Vec::new();
        let mut hrefs: Vec<String> = Vec::new();
        let text = |content: &str, marks: &[Mark], href: Option<&String>| Inline::Text {
            text: content.to_owned(),
            marks: marks.iter().copied().collect::<Marks>(),
            href: href.cloned(),
        };
        for token in children {
            let href = hrefs.last();
            match token.kind {
                Kind::Text => items.push(text(&token.content, &marks, href)),
                Kind::CodeInline => {
                    let mut with = marks.clone();
                    with.push(Mark::Code);
                    items.push(text(&token.content, &with, href));
                }
                Kind::Softbreak => items.push(text(" ", &marks, href)),
                Kind::Hardbreak => items.push(Inline::Break),
                Kind::HtmlInline if is_br(&token.content) => items.push(Inline::Break),
                Kind::HtmlInline => items.push(Inline::Html(token.content.clone())),
                Kind::StrongOpen => marks.push(Mark::Strong),
                Kind::EmOpen => marks.push(Mark::Em),
                Kind::SOpen => marks.push(Mark::Strike),
                Kind::StrongClose | Kind::EmClose | Kind::SClose => {
                    marks.pop();
                }
                Kind::LinkOpen => hrefs.push(match token.attr("href") {
                    Some(Value::Text(h)) => h.clone(),
                    _ => String::new(),
                }),
                Kind::LinkClose => {
                    hrefs.pop();
                }
                Kind::Image => {
                    let alt: String = token
                        .children
                        .as_deref()
                        .unwrap_or(&[])
                        .iter()
                        .filter(|c| matches!(c.kind, Kind::Text | Kind::TextSpecial | Kind::CodeInline))
                        .map(|c| c.content.as_str())
                        .collect();
                    let attr = |name: &str| match token.attr(name) {
                        Some(Value::Text(v)) => Some(v.clone()),
                        _ => None,
                    };
                    items.push(Inline::Image {
                        alt,
                        src: attr("src").unwrap_or_default(),
                        title: attr("title"),
                        href: href.cloned(),
                    });
                }
                Kind::FootnoteRef => items.push(Inline::NoteRef(self.label(token)?)),
                _ => {}
            }
        }
        Ok(items)
    }
}

/// `text-align:\s*(left|center|right)` in a cell's style.
fn align_of(style: &str) -> Option<String> {
    let mut rest = style;
    while let Some(at) = rest.find("text-align:") {
        let after = rest[at + "text-align:".len()..].trim_start_matches(py_space);
        for value in ["left", "center", "right"] {
            if after.starts_with(value) {
                return Some(value.to_owned());
            }
        }
        rest = &rest[at + 1..];
    }
    None
}

/// Whether a cell's inline content says anything: text, or anything but text.
fn has_text(inline: &[Inline]) -> bool {
    inline.iter().any(|node| match node {
        Inline::Text { text, .. } => !py_strip(text).is_empty(),
        _ => true,
    })
}

/// The length of a `<br\s*/?>` (any case) at the start of `text`.
fn br_at(text: &str) -> Option<usize> {
    let lower = text.get(..3)?;
    if !lower.eq_ignore_ascii_case("<br") {
        return None;
    }
    let rest = &text[3..];
    let spaced = rest.trim_start_matches(py_space);
    let mut used = 3 + rest.len() - spaced.len();
    let spaced = match spaced.strip_prefix('/') {
        Some(s) => {
            used += 1;
            s
        }
        None => spaced,
    };
    spaced.starts_with('>').then_some(used + 1)
}

/// `<br>`, `<br/>`, `<br />` in any case, alone.
fn is_br(text: &str) -> bool {
    br_at(text) == Some(text.len())
}

/// How many `<br>` an HTML block is when it is nothing else (white space around them).
fn breaks_only(content: &str) -> usize {
    let mut rest = content.trim_start_matches(py_space);
    let mut count = 0;
    while let Some(used) = br_at(rest) {
        count += 1;
        rest = rest[used..].trim_start_matches(py_space);
    }
    if rest.is_empty() {
        count
    } else {
        0
    }
}

/// An HTML block: a paragraph of line breaks, its comments, or raw HTML.
fn html_block(content: &str) -> Vec<Block> {
    let breaks = breaks_only(content);
    if breaks > 0 {
        return vec![Block::Paragraph(vec![Inline::Break; breaks])];
    }
    let stripped = py_strip(content);
    let mut comments = Vec::new();
    let mut outside = String::new();
    let mut rest = stripped;
    while let Some(at) = rest.find("<!--") {
        let Some(end) = rest[at + 4..].find("-->") else { break };
        outside.push_str(&rest[..at]);
        comments.push(py_strip(&rest[at + 4..at + 4 + end]).to_owned());
        rest = &rest[at + 4 + end + 3..];
    }
    outside.push_str(rest);
    if !comments.is_empty() && py_strip(&outside).is_empty() {
        return comments.into_iter().map(Block::Comment).collect();
    }
    vec![Block::Html(content.to_owned())]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaks_and_comments() {
        assert_eq!(breaks_only(" <br> <BR/>\n<br />\n"), 3);
        assert_eq!(breaks_only("<br> x"), 0);
        assert_eq!(html_block("<!-- a --> <!--b-->\n"), vec![Block::Comment("a".into()), Block::Comment("b".into())]);
        assert_eq!(align_of("text-align:center"), Some("center".into()));
    }

    #[test]
    fn lists_and_tables() {
        let blocks = parse_model("3. a\n4. b\n\n| | |\n|-|-|\n| x | y |\n").unwrap_or_default();
        assert!(matches!(&blocks[0], Block::List(l) if l.ordered && l.start == 3 && l.items.len() == 2));
        assert!(matches!(&blocks[1], Block::Table(t) if !t.headed && t.rows.len() == 1));
    }
}
