//! Owns: a body paragraph the layout cannot lay out taken into the flow as a placeholder
//! (docx2svg's `paginate.placeholder_item`, with `CARRY_ON`): [`crate::placeholder`]'s
//! estimated lines in a paragraph item that keeps nothing with it.
//!
//! Must not: measure the paragraph's text.

use super::{int, stop, FlowItem, FlowMaker, NoteForms, ParaItem};
use crate::placeholder::{placeholder_measured, Placeholder};
use crate::px::Px;
use ooxml_word_model::model::Paragraph;
use std::sync::Arc;

impl FlowMaker<'_, '_> {
    /// `paragraph` as a placeholder (`placeholder_item`): its estimated lines, its own space
    /// before and after, nothing kept with it, no widow control; a drop cap that drops into
    /// it is lost with it.
    pub(super) fn placeholder_item(
        &mut self,
        paragraph: &Arc<Paragraph>,
        (body, block, section, section_start): (usize, usize, usize, bool),
        placeholder: Placeholder,
    ) -> FlowItem {
        self.pending_drop = None;
        let Some(geometry) = self.cx.sections.get(section).cloned() else {
            return stop(block, body, section, "no section");
        };
        let measured = placeholder_measured(self.cx.document, self.model, paragraph, &geometry);
        let pp = self.props_of_block(block).unwrap_or_default();
        FlowItem::Para(Arc::new(ParaItem {
            block,
            body,
            section,
            measured: Arc::new(measured),
            before: int(&pp, "spacing.before"),
            after: int(&pp, "spacing.after"),
            contextual: false,
            style: None,
            border: Px::ZERO,
            top_border: Px::ZERO,
            keep_next: false,
            keep_lines: false,
            page_break_before: false,
            widow: Some(false),
            section_start,
            section_mark: false,
            notes: Vec::new(),
            role: self.cx.roles.get(body).copied().flatten(),
            continuation: None,
            anchors: Vec::new(),
            source: None,
            drop_cap: None,
            note_forms: NoteForms::default(),
            placeholder: Some(Arc::new(placeholder)),
        }))
    }
}
