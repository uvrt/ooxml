//! Owns: markdown-it's core rules around the inline parser: `normalize` (line ends and NUL)
//! and `text_join` (escaped characters joined into the text around them).
//!
//! Must not: parse.

use crate::token::{Kind, Token};

/// `\r\n` and `\r` as `\n`; NUL as U+FFFD.
pub(crate) fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\0' => out.push('\u{fffd}'),
            other => out.push(other),
        }
    }
    out
}

/// Escaped characters become text, and adjacent texts one.
pub(crate) fn text_join(tokens: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    for mut token in tokens {
        if token.kind == Kind::TextSpecial {
            token.kind = Kind::Text;
        }
        match out.last_mut() {
            Some(last) if token.kind == Kind::Text && last.kind == Kind::Text => last.content.push_str(&token.content),
            _ => out.push(token),
        }
    }
    out
}

/// Join escapes into text in each inline token's children, and in theirs (`text_join`).
pub(crate) fn join_inline(tokens: &mut [Token]) {
    for token in tokens.iter_mut().filter(|t| t.kind == Kind::Inline) {
        if let Some(children) = token.children.take() {
            let mut joined = text_join(children);
            for child in &mut joined {
                if let Some(grand) = child.children.take() {
                    child.children = Some(text_join(grand));
                }
            }
            token.children = Some(joined);
        }
    }
}

/// Footnote definitions moved to a block at the end, in the order they are referenced,
/// each with its back references (markdown-it-footnote's `footnote_tail`).
pub(crate) fn footnote_tail(tokens: Vec<Token>, env: &mut crate::env::Env) -> Vec<Token> {
    use crate::token::Note;
    use std::collections::HashMap;
    let mut kept = Vec::with_capacity(tokens.len());
    let mut definitions: HashMap<String, Vec<Token>> = HashMap::new();
    let mut current: Option<(String, Vec<Token>)> = None;
    for token in tokens {
        match token.kind {
            Kind::FootnoteReferenceOpen => {
                let label = token.meta.and_then(|m| m.label).unwrap_or_default();
                current = Some((label, Vec::new()));
            }
            Kind::FootnoteReferenceClose => {
                if let Some((label, body)) = current.take() {
                    definitions.insert(format!(":{label}"), body);
                }
            }
            _ => match current.as_mut() {
                Some((_, body)) => body.push(token),
                None => kept.push(token),
            },
        }
    }
    if env.footnotes.is_empty() {
        return kept;
    }
    kept.push(Token::new(Kind::FootnoteBlockOpen, "", 1));
    for (i, note) in std::mem::take(&mut env.footnotes).into_iter().enumerate() {
        let mut open = Token::new(Kind::FootnoteOpen, "", 1);
        open.meta = Some(Note {
            id: Some(i),
            sub_id: None,
            label: note.label.clone(),
        });
        kept.push(open);
        if let Some((content, children)) = note.inline {
            let mut p = Token::new(Kind::ParagraphOpen, "p", 1);
            p.block = true;
            kept.push(p);
            let mut inline = Token::new(Kind::Inline, "", 0);
            inline.children = Some(children);
            inline.content = content;
            kept.push(inline);
            let mut close = Token::new(Kind::ParagraphClose, "p", -1);
            close.block = true;
            kept.push(close);
        } else if let Some(label) = &note.label {
            kept.extend(definitions.get(&format!(":{label}")).cloned().unwrap_or_default());
        }
        let last = kept.pop_if(|t| t.kind == Kind::ParagraphClose);
        for j in 0..note.count.max(1) {
            let mut anchor = Token::new(Kind::FootnoteAnchor, "", 0);
            anchor.meta = Some(Note {
                id: Some(i),
                sub_id: Some(j),
                label: note.label.clone(),
            });
            kept.push(anchor);
        }
        kept.extend(last);
        kept.push(Token::new(Kind::FootnoteClose, "", -1));
    }
    kept.push(Token::new(Kind::FootnoteBlockClose, "", -1));
    kept
}
