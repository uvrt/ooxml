//! Owns: the size a comment's reference run is written at (docx-agent's
//! `_reference_size`): the size, in half-points, of the text the reference follows -- the
//! character before the place, else the paragraph's first -- as Word applies it there,
//! the paragraph's table cell included (Word writes the size on the reference run;
//! measured).
//!
//! Must not: change anything.

use crate::text::atoms;
use crate::work::Work;
use ooxml_word_model::docx::cells::{model_path, TableFacts};
use ooxml_word_model::docx::formatting::{Cell, Formatting};
use ooxml_word_model::docx::Docx;
use ooxml_xml::Element;
use std::sync::Arc;

/// What the sizes of one edit are read with: the document's formatting, read once.
pub(crate) struct Sizes<'d> {
    docx: &'d Docx,
    formatting: Formatting,
}

impl<'d> Sizes<'d> {
    /// The sizes of `docx`.
    pub(crate) fn of(docx: &'d Docx) -> Self {
        Sizes {
            docx,
            formatting: Formatting::read(docx),
        }
    }

    /// The size of the text before character `offset` of `paragraph` (at `path` of `part`),
    /// or of its first character when the place opens it; `None` where nothing resolves.
    pub(crate) fn at(&self, part: &str, path: &[u32], paragraph: &Arc<Element>, offset: usize) -> Option<i64> {
        let work = Work::of(paragraph, Vec::new());
        let items = atoms(&work, work.root());
        let run = if offset > 0 && offset <= items.len() {
            items.get(offset - 1).and_then(|a| a.run)
        } else {
            items.first().and_then(|a| a.run)
        };
        let element = run.and_then(|r| work.build_element(r));
        // docx-agent's `effective.run`: the run's first character decides the face
        let first = run
            .and_then(|r| items.iter().find(|a| a.run == Some(r)))
            .map_or('a', |a| a.ch);
        let cell = self.cell(part, path);
        let effective = self
            .formatting
            .run(paragraph, element.as_deref().map(|e| e as &Element), first, cell.as_ref());
        // Python's round(): halves to even
        let doubled = effective.size * 2.0;
        #[allow(clippy::cast_possible_truncation)]
        Some(doubled.round_ties_even() as i64)
    }

    /// Where the paragraph at `path` of `part` stands in a table, as the layout model reads
    /// its outermost table.
    fn cell(&self, part: &str, path: &[u32]) -> Option<Cell> {
        let story = self.docx.part(part)?;
        let (table, at) = (1..path.len()).find_map(|n| {
            let prefix = path.get(..n)?;
            story.ids.table_at(prefix).map(|t| (t, n))
        })?;
        let facts = TableFacts::read(&self.formatting, &table.element);
        let below = path.get(at..)?;
        model_path(&table.element, below).and_then(|p| facts.cell(&p).cloned())
    }
}
