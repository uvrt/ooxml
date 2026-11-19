//! Owns: [`InsertMarkdown`], Markdown written as the document's own blocks, as docx-agent's
//! `markdown/write.py` `insert_markdown` writes it: the text read into the model
//! ([`parse`]), each construct written in the style the [`WriteMap`] names for it
//! ([`writer`], [`lists`], [`table`]), put at its place -- a story's end (a body that is one
//! empty paragraph typed into: the first new paragraph keeps its ids), after or before a
//! block -- with a separator paragraph between two tables and a paragraph after a table
//! that ends its container, stamped, and tracked as one revision group ([`insertion`]).
//!
//! Not ported yet ([`EditError::Unsupported`]: the host writes them its own way):
//! pictures, footnotes, `replace:`, and the `like` and `table_like` looks.
//!
//! Must not: write but through the sink, or know a tool's JSON.

mod insertion;
pub(crate) use insertion::insert_content;
mod lists;
mod parse;
mod table;
mod writer;

use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::markdown::model::{Block, Inline};
pub use ooxml_word_model::markdown::stylemap::WriteMap;

/// How a Markdown text is written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Raw HTML kept as its text (`html="text"`); else refused.
    pub html_as_text: bool,
    /// Emphasis and strong as direct `w:i`/`w:b` (`emphasis="direct"`); else their styles.
    pub emphasis_direct: bool,
}

/// Write Markdown as blocks.
#[derive(Debug, Clone)]
pub struct InsertMarkdown {
    /// The Markdown.
    pub markdown: String,
    /// `end`, `end:<story>`, `after:<id>`, `before:<id>` or a bare id (after it).
    pub at: String,
    /// The style each construct is written in.
    pub style_map: WriteMap,
    /// How it is written.
    pub options: Options,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What an insert_markdown did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkdownInserted {
    /// The first block made.
    pub first: Option<String>,
    /// Whether anything was written.
    pub changed: bool,
    /// Every block made, in document order (a table, then its cells' paragraphs).
    pub created: Vec<String>,
    /// Ids the edit's stamping renamed, old to new.
    pub renamed: Vec<(String, String)>,
    /// Blocks removed (the empty paragraph typed into).
    pub removed: Vec<String>,
    /// Styles added, alignment written directly.
    pub warnings: Vec<String>,
    /// The top-level blocks made.
    pub blocks: Vec<String>,
}

impl Command<WordDoc> for InsertMarkdown {
    type Out = MarkdownInserted;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let blocks = self.read()?;
        let docx = doc.docx()?;
        insertion::locate(doc, &docx, &self.at, &blocks).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<MarkdownInserted> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let blocks = self.read()?;
        if !blocks.iter().any(produces) {
            return Ok(MarkdownInserted::default());
        }
        let out = insertion::insert(&mut sink, self, &blocks)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl InsertMarkdown {
    /// The blocks to write, checked before anything changes (`_check`, `_pictures`).
    fn read(&self) -> Result<Vec<Block>> {
        let blocks: Vec<Block> = parse::parse_model(&self.markdown)?
            .into_iter()
            .filter(|b| !matches!(b, Block::Footnote(..) | Block::Comment(_)))
            .collect();
        if !self.options.html_as_text {
            let mut all = Vec::new();
            writer::walk_blocks(&blocks, &mut all);
            if let Some(Block::Html(text)) = all.iter().find(|b| matches!(b, Block::Html(_))) {
                return Err(html_refused(parse::py_strip(text)));
            }
            let mut inline = Vec::new();
            writer::walk_inline(&blocks, &mut inline);
            if let Some(Inline::Html(text)) = inline
                .iter()
                .find(|i| matches!(i, Inline::Html(t) if !t.starts_with("<!--")))
            {
                return Err(html_refused(text));
            }
        }
        let mut inline = Vec::new();
        writer::walk_inline(&blocks, &mut inline);
        if inline.iter().any(|i| matches!(i, Inline::NoteRef(_))) {
            return Err(EditError::Unsupported("a footnote in Markdown".to_owned()));
        }
        if inline.iter().any(|i| matches!(i, Inline::Image { .. })) {
            return Err(EditError::Unsupported("a picture in Markdown".to_owned()));
        }
        Ok(blocks)
    }
}

fn html_refused(text: &str) -> EditError {
    let shown: String = text.chars().take(60).collect();
    EditError::Refused(format!(
        "raw HTML is not inserted (html='text' keeps it as text): {shown:?}"
    ))
}

/// Whether writing `block` makes any element.
fn produces(block: &Block) -> bool {
    match block {
        Block::Paragraph(inline) => !inline.is_empty(),
        Block::Comment(_) | Block::Footnote(..) => false,
        Block::Quote(inner) => inner.iter().any(produces),
        _ => true,
    }
}

/// `urllib.parse.unquote`: `%XX` sequences decoded as UTF-8, invalid bytes replaced.
pub(crate) fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut k = 0;
    while let Some(&b) = bytes.get(k) {
        let hex = |i: usize| bytes.get(i).and_then(|c| char::from(*c).to_digit(16));
        match (b, hex(k + 1), hex(k + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
                k += 3;
            }
            _ => {
                out.push(b);
                k += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent() {
        assert_eq!(percent_decode("a%20b%C3%A9%zz"), "a b\u{e9}%zz");
    }
}
