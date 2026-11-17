//! Owns: what the flow holds of a paragraph's notes (docx2svg's `paginate.flow`, its
//! notes' part): its footnotes by line and its section's separators, an endnote's
//! continuation line, and in mode 15 under columns each footnote as a flow of its own
//! (what a page's note area is laid out from).
//!
//! Must not: measure a paragraph's own lines, or place anything.

use super::{FlowContext, FlowMaker, Labels};
use crate::notes::lines::NoteContext;
use crate::notes::{NoteLines, NoteRole};
use crate::paragraph::MeasuredParagraph;
use crate::px::Px;
use crate::sections::Sections;
use ooxml_word_model::model::{Block, Paragraph};
use std::sync::Arc;

/// A paragraph's footnotes by line, and an endnote's continuation line.
pub(super) struct ParaNotes {
    pub(super) notes: Vec<(usize, NoteLines)>,
    pub(super) continuation: Option<Px>,
}

impl FlowMaker<'_, '_> {
    /// A paragraph's footnotes by line (and its section's separators, where it has any),
    /// and an endnote's continuation line.
    pub(super) fn paragraph_notes(
        &mut self,
        paragraph: &Paragraph,
        measured: &MeasuredParagraph,
        section: usize,
        role: Option<NoteRole>,
    ) -> Result<ParaNotes, crate::notes::flowed::Unlaid> {
        let Some(geometry) = self.cx.sections.get(section).cloned() else {
            return Ok(ParaNotes {
                notes: Vec::new(),
                continuation: None,
            });
        };
        let mut cx = NoteContext {
            document: self.cx.document,
            faces: self.cx.faces,
            locale: self.cx.locale,
            measurer: &mut self.state.measurer,
            model: self.model,
        };
        let lines = (measured.measured.pieces.as_slice(), measured.measured.lines.as_slice());
        let notes = self.notes.notes_by_line(&mut cx, paragraph, lines, (section, &geometry))?;
        if !notes.is_empty() && !self.separators.contains_key(&section) {
            let found = self.notes.separators(&mut cx, (section, &geometry));
            self.separators.insert(section, found);
        }
        let continuation = match role {
            Some(NoteRole::Endnote) => self.notes.continuation(&mut cx, (section, &geometry)),
            _ => None,
        };
        let mode15 = crate::input::rules(self.cx.document).mode15();
        let referenced = crate::notes::flowed::references(paragraph).into_iter().map(|(_, n)| n);
        if mode15 && geometry.multi_column() {
            for note in referenced {
                self.note_flow(note, section, &geometry);
            }
        } else if geometry.unequal_columns() {
            // A column's foot holds a note broken in its own width.
            let widths: std::collections::BTreeSet<i64> = geometry.column_boxes().iter().map(|b| b.width).collect();
            for note in referenced {
                for width in &widths {
                    let mut cx = NoteContext {
                        document: self.cx.document,
                        faces: self.cx.faces,
                        locale: self.cx.locale,
                        measurer: &mut self.state.measurer,
                        model: &mut *self.model,
                    };
                    if let Some(lines) = self.notes.note(&mut cx, note, (section, &geometry), Some(*width)) {
                        self.notes_at.insert((note, section, *width), lines);
                    }
                }
            }
        }
        Ok(ParaNotes { notes, continuation })
    }

    /// Footnote `note`'s paragraphs as a flow in section `section`'s columns (a section of
    /// one section, no stories), kept for the flow (`note_area`'s `flow(story)`).
    fn note_flow(&mut self, note: i64, section: usize, geometry: &crate::sections::SectionGeometry) {
        if self.note_items.contains_key(&(note, section)) {
            return;
        }
        let Some(blocks) = self.cx.document.footnotes.notes.get(&note).cloned() else {
            return;
        };
        let paragraphs = blocks.iter().filter(|b| matches!(b, Block::Paragraph(_))).count();
        let sections = Sections::new(&[crate::notes::flowed::note_section(&geometry.props, paragraphs)], false);
        let cx = FlowContext {
            document: self.cx.document,
            blocks: &blocks,
            sections: &sections,
            labels: Labels::None,
            faces: self.cx.faces,
            locale: self.cx.locale,
            tables: self.cx.tables,
            roles: &[],
        };
        let flow = FlowMaker::new(&cx, &mut *self.model, &mut *self.state).build();
        self.note_items.insert((note, section), Arc::from(flow.items));
    }
}
