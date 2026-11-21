//! Owns: the top-level blocks [`super::InsertMarkdown`] makes of a Markdown text, by kind
//! and in order, without writing them (docx-agent's `field_text.shape` and
//! `markdown_shape`): what lines a value's share of a call's Markdown up with the blocks
//! the call made.
//!
//! Must not: write a document, or count what the writer does not make (footnote
//! definitions, comments, an empty paragraph).

use super::parse::parse_model;
use crate::Result;
use ooxml_word_model::markdown::model::Block;

/// A top-level block the Markdown writer makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Made {
    /// A paragraph (`p`).
    Paragraph,
    /// A table (`t`).
    Table,
}

/// The top-level blocks `text` is written as, in order -- a separator paragraph between two
/// tables included.
///
/// # Errors
/// What the Markdown reader refuses (an inline footnote).
pub fn markdown_shape(text: &str) -> Result<Vec<Made>> {
    let blocks: Vec<Block> = parse_model(text)?
        .into_iter()
        .filter(|b| !matches!(b, Block::Footnote(..) | Block::Comment(_)))
        .collect();
    Ok(shape(&blocks))
}

fn shape(blocks: &[Block]) -> Vec<Made> {
    let mut out: Vec<Made> = Vec::new();
    for block in blocks {
        let made = one(block);
        if made.first() == Some(&Made::Table) && out.last() == Some(&Made::Table) {
            out.push(Made::Paragraph);
        }
        out.extend(made);
    }
    out
}

fn one(block: &Block) -> Vec<Made> {
    match block {
        Block::Paragraph(inline) if inline.is_empty() => Vec::new(),
        Block::Paragraph(_) | Block::Heading(..) | Block::Rule => vec![Made::Paragraph],
        Block::Code(text) => {
            let body = text.strip_suffix('\n').unwrap_or(text);
            vec![Made::Paragraph; body.split('\n').count()]
        }
        Block::Quote(inner) => shape(inner),
        Block::List(list) => {
            let mut out = Vec::new();
            for item in &list.items {
                let rest = match item.first() {
                    Some(Block::Paragraph(_)) => item.get(1..).unwrap_or_default(),
                    _ => item.as_slice(),
                };
                out.push(Made::Paragraph);
                for inner in rest {
                    out.extend(one(inner));
                }
            }
            out
        }
        Block::Table(_) => vec![Made::Table],
        Block::Html(text) => vec![Made::Paragraph; text.trim_end_matches('\n').split('\n').count()],
        Block::Comment(_) | Block::Footnote(..) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Made::{Paragraph as P, Table as T};

    #[test]
    fn paragraphs_lists_code_and_tables() {
        let shape = |text: &str| markdown_shape(text).unwrap();
        assert_eq!(shape("# Title\n\nSome text."), [P, P]);
        assert_eq!(shape("- one\n- two\n  - nested"), [P, P, P]);
        assert_eq!(shape("```\na\nb\n```"), [P, P]);
        assert_eq!(shape("| a |\n|---|\n| 1 |\n\n| b |\n|---|\n| 2 |"), [T, P, T]);
        assert_eq!(shape("> quoted\n\n---"), [P, P]);
        assert_eq!(shape("text[^1]\n\n[^1]: a note"), [P]);
        assert!(shape("").is_empty());
    }
}
