//! Owns: [`InsertMarkdown`], Markdown written as the document's own blocks, as docx-agent's
//! `markdown/write.py` `insert_markdown` writes it: the text read into the model
//! ([`parse`]), each construct written in the style the [`WriteMap`] names for it
//! ([`writer`], [`lists`], [`table`]), put at its place -- a story's end (a body that is one
//! empty paragraph typed into: the first new paragraph keeps its ids), after or before a
//! block, or in place of blocks (`replace:`, the new paragraphs keeping the replaced ones'
//! settings: [`keep`]; list items given a look, [`like`], are not matched to them) -- with a separator paragraph between two tables and a paragraph
//! after a table that ends its container, stamped, and tracked as one revision group
//! ([`insertion`]); a table cell style whose indents do not fit its cells said in a warning
//! ([`cell_indents`]); list items like given paragraphs by level ([`like`]: the sample's
//! style, list at its level, indents and page settings).
//!
//! Not ported yet ([`EditError::Unsupported`]: the host writes them its own way):
//! pictures, footnotes, and tables in a given table's frame (`table_like`).
//!
//! Must not: write but through the sink, or know a tool's JSON.

mod cell_indents;
mod insertion;
mod keep;
mod like;
pub(crate) use insertion::{insert_content, track, track_table};
pub use like::{LevelLook, ListKind};
mod lists;
mod parse;
pub(crate) use parse::py_space;
mod shape;
pub use shape::{markdown_shape, Made};
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
    /// The looks list items take, by level (`like`); none: each list's own.
    pub like: Vec<LevelLook>,
}

/// Write Markdown as blocks.
#[derive(Debug, Clone)]
pub struct InsertMarkdown {
    /// The Markdown.
    pub markdown: String,
    /// `end`, `end:<story>`, `after:<id>`, `before:<id>` or a bare id (after it).
    pub at: ooxml_word_model::address::Place,
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
    /// Blocks removed (the empty paragraph typed into).
    pub removed: Vec<String>,
    /// Styles added, alignment written directly.
    pub warnings: Vec<String>,
    /// The top-level blocks made.
    pub blocks: Vec<String>,
    /// At `replace:`, what each new paragraph that kept any of the replaced ones' settings
    /// kept, in order.
    pub kept: Vec<KeptSettings>,
}

/// What a new paragraph kept of a paragraph it replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptSettings {
    /// The new paragraph.
    pub block: String,
    /// The replaced paragraph whose settings it took.
    pub from: String,
    /// The settings, as a result names them (`style Report Body`, `keep_with_next`,
    /// `list level 0`).
    pub settings: Vec<String>,
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
        // in place of blocks, nothing written still takes them away
        let replaces = matches!(self.at.kind(), ooxml_word_model::address::PlaceKind::Replace(_));
        if !replaces && !blocks.iter().any(produces) {
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
        "raw HTML is not inserted (html='text' keeps it as text): {}",
        ooxml_base::pyfmt::repr_str(&shown)
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
