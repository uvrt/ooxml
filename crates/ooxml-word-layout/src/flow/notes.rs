//! Owns: what the flow holds of a paragraph's notes (docx2svg's `paginate.flow`, its
//! notes' part): its footnotes by line and its section's separators, an endnote's
//! continuation line, and in mode 15 under columns each footnote as a flow of its own
//! (what a page's note area is laid out from).
//!
//! Must not: measure a paragraph's own lines, or place anything.

use super::{FlowContext, FlowItem, FlowMaker, Labels};
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
    pub(super) forms: NoteForms,
}

/// What a page lays a paragraph's footnotes out from beyond their lines: in mode 15 under
/// several columns each note's paragraphs as a flow of their own, below it in unequal
/// columns each note's lines in each column width. Part of the paragraph as the flow
/// holds it, so that a relayout sees a change to any of them.
#[derive(Debug, Clone, Default)]
pub struct NoteForms {
    /// The notes' flows.
    pub flows: Vec<Arc<[FlowItem]>>,
    /// The notes' lines in the columns' widths.
    pub widths: Vec<NoteLines>,
}

impl PartialEq for NoteForms {
    fn eq(&self, other: &NoteForms) -> bool {
        let same_flow = |a: &Arc<[FlowItem]>, b: &Arc<[FlowItem]>| {
            Arc::ptr_eq(a, b)
                || (a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| crate::incremental::same_item(x, y)))
        };
        self.widths == other.widths
            && self.flows.len() == other.flows.len()
            && self.flows.iter().zip(&other.flows).all(|(a, b)| same_flow(a, b))
    }
}

impl Eq for NoteForms {}

/// The footnotes referenced in a table's cells (`cell_notes`): per cell paragraph (its
/// index among the table's paragraphs) each reference's text index and note, and each
/// note's lines laid out in the section's width.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CellNotes {
    /// Cell paragraph -> its references `(text index, note)`, in order.
    pub references: std::collections::BTreeMap<usize, Vec<(i64, i64)>>,
    /// Note -> its lines.
    pub lines: std::collections::BTreeMap<i64, NoteLines>,
}

impl CellNotes {
    /// Whether no cell references a note.
    pub fn is_empty(&self) -> bool {
        self.references.is_empty()
    }
}

impl FlowMaker<'_, '_> {
    /// The footnotes referenced in the cells of the table at top-level block `body`
    /// (its first paragraph `block`), in section `section`; its section's separators
    /// measured where it has any.
    pub(super) fn cell_notes(&mut self, block: usize, body: usize, section: usize) -> Arc<CellNotes> {
        let mut out = CellNotes::default();
        let count = self.paragraphs_in(body);
        for k in 0..count {
            let Some(paragraph) = self.paragraphs.get(block + k) else {
                continue;
            };
            let found = crate::notes::flowed::references(paragraph);
            if !found.is_empty() {
                out.references.insert(k, found);
            }
        }
        let Some(geometry) = self.cx.sections.get(section).cloned().filter(|_| !out.is_empty()) else {
            return Arc::new(out);
        };
        let mut cx = NoteContext {
            document: self.cx.document,
            faces: self.cx.faces,
            locale: self.cx.locale,
            measurer: &mut self.state.measurer,
            model: self.model,
        };
        let ids: std::collections::BTreeSet<i64> = out.references.values().flatten().map(|(_, n)| *n).collect();
        for id in ids {
            if let Some(lines) = self.notes.note(&mut cx, id, (section, &geometry), None) {
                out.lines.insert(id, lines);
            }
        }
        if !self.separators.contains_key(&section) {
            let found = self.notes.separators(&mut cx, (section, &geometry));
            self.separators.insert(section, found);
        }
        Arc::new(out)
    }

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
                forms: NoteForms::default(),
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
        let notes = self
            .notes
            .notes_by_line(&mut cx, paragraph, lines, (section, &geometry))?;
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
        let mut forms = NoteForms::default();
        if mode15 && geometry.multi_column() {
            for note in referenced {
                self.note_flow(note, section, &geometry);
                if let Some(flow) = self.note_items.get(&(note, section)) {
                    forms.flows.push(Arc::clone(flow));
                }
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
                        forms.widths.push(NoteLines::clone(&lines));
                        self.notes_at.insert((note, section, *width), lines);
                    }
                }
            }
        }
        Ok(ParaNotes {
            notes,
            continuation,
            forms,
        })
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
        let sections = Sections::new(
            &[crate::notes::flowed::note_section(&geometry.props, paragraphs)],
            false,
        );
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
