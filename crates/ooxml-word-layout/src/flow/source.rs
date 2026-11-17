//! Owns: which paragraphs of the flow keep what their lines are broken from, so that the
//! paginator can break them again (docx2svg's `paginate.flow`, its `ParaSource` part): in
//! a body with drawings text wraps beside or floating tables, in a section of unequal
//! columns, and where Word moves only a hyphenated word to the next page.
//!
//! Must not: break a paragraph again (`crate::rebreak` does), or place anything.

use super::FlowMaker;
use crate::floating::SIDE_WRAPS;
use crate::notes::flowed::references;
use crate::paginate::HyphenMove;
use crate::paragraph::MeasuredParagraph;
use crate::rebreak::ParaSource;
use ooxml_word_model::model::{Block, Paragraph};
use std::sync::Arc;

/// Whether a body's paragraphs may be broken beside something on their page: a drawing
/// text wraps beside, or a floating table (`Flow.wraps`).
pub(super) fn wraps(body: &[Block]) -> bool {
    body.iter().any(|block| match block {
        Block::Paragraph(p) => p.runs.iter().any(|r| {
            r.anchors
                .iter()
                .any(|a| a.wrap.as_deref().is_some_and(|w| SIDE_WRAPS.contains(&w)))
        }),
        Block::Table(t) => t.properties.contains_key("tblpPr"),
    })
}

impl FlowMaker<'_, '_> {
    /// What the paragraph's lines are broken from, where the paginator may break them
    /// again.
    pub(super) fn para_source(
        &mut self,
        paragraph: &Arc<Paragraph>,
        measured: &MeasuredParagraph,
        section: usize,
    ) -> Option<Arc<ParaSource>> {
        let geometry = self.cx.sections.get(section).cloned()?;
        let unequal = geometry.unequal_columns();
        let word_moves = !measured.auto_hyphen_ends.is_empty()
            && crate::engine::hyphen_at_page_end(self.cx.document) == HyphenMove::Word;
        if !(self.wraps || unequal || word_moves) {
            return None;
        }
        let resolved = self.model.resolve_paragraph(self.cx.document, paragraph);
        let lines_at = crate::input::section_geometry(self.cx.document, &resolved.paragraph, &geometry);
        let mut notes = Vec::new();
        for (at, note) in references(paragraph) {
            let mut cx = crate::notes::lines::NoteContext {
                document: self.cx.document,
                faces: self.cx.faces,
                locale: self.cx.locale,
                measurer: &mut self.state.measurer,
                model: &mut *self.model,
            };
            if let Some(laid) = self.notes.note(&mut cx, note, (section, &geometry), None) {
                notes.push((at, laid));
            }
        }
        Some(Arc::new(ParaSource {
            paragraph: Arc::clone(paragraph),
            resolved,
            geometry: lines_at,
            columns: geometry.multi_column(),
            section: unequal.then_some(geometry),
            notes,
        }))
    }
}
