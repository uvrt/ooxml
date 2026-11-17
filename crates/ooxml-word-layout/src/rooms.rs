//! Owns: the engine's [`TableRooms`] -- a flow's top-level tables laid out again, while
//! its pages are laid out, in another room than their column (docx2svg's `TableItem.room`
//! and `reflow`): beside a drawing text wraps around, or in a column right of their
//! section's first. Each table is laid out as the flow laid it out (the table layout, the
//! paragraph before it, its cells' labels), in a section of one column whose left margin
//! is moved to the room's left edge and whose width is the room's; each room once per
//! pass.
//!
//! Must not: decide which room a table goes into (the paginator does), or keep anything
//! beyond one pass.

use crate::faces::Faces;
use crate::flow::FlowItem;
use crate::sections::{SectionGeometry, Sections};
use crate::table::{TableContext, TableJob, TableLayout, TableRoom, TableRooms};
use ooxml_base::Locale;
use ooxml_text::Measurer;
use ooxml_word_model::model::Block;
use ooxml_word_model::{Model, WordDocument};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// What laying a table out again reads: the version, its flow, sections and labels, the
/// table layout, the faces and the locale.
pub struct RoomSource<'a> {
    /// The version laid out.
    pub document: &'a WordDocument,
    /// Its flow's items.
    pub items: &'a [FlowItem],
    /// Its sections.
    pub sections: &'a Sections,
    /// Its paragraphs' list labels, document order (cells included).
    pub labels: &'a [Option<String>],
    /// The table layout.
    pub tables: &'a dyn TableLayout,
    /// The faces.
    pub faces: &'a dyn Faces,
    /// The locale measures are keyed under.
    pub locale: &'a Locale,
}

/// A flow's tables laid out in other rooms, through the engine's model and measures.
pub struct Rooms<'a> {
    source: RoomSource<'a>,
    work: RefCell<(&'a mut Model, &'a mut Measurer)>,
    found: RefCell<HashMap<(usize, i64, i64), Option<TableRoom>>>,
}

impl<'a> Rooms<'a> {
    /// The rooms of `source`'s tables, laid out with `model` and `measurer`.
    pub fn new(source: RoomSource<'a>, model: &'a mut Model, measurer: &'a mut Measurer) -> Rooms<'a> {
        Rooms {
            source,
            work: RefCell::new((model, measurer)),
            found: RefCell::default(),
        }
    }

    fn lay_out(&self, item: usize, left: i64, width: i64) -> Option<TableRoom> {
        let src = &self.source;
        let Some(FlowItem::Table(t)) = src.items.get(item) else {
            return None;
        };
        let Some(Block::Table(table)) = src.document.body.get(t.body) else {
            return None;
        };
        let previous = match t.body.checked_sub(1).and_then(|k| src.document.body.get(k)) {
            Some(Block::Paragraph(p)) => Some(Arc::clone(p)),
            _ => None,
        };
        let section = src.sections.get(t.section)?;
        let mut props = (*section.props).clone();
        let new_left = props.margins.left + left;
        props.margins.right = props.page_size.width - new_left - props.margins.gutter - width;
        props.margins.left = new_left;
        props.column_count = 1;
        props.columns = Arc::from(Vec::new());
        let geometry = SectionGeometry {
            props: Arc::new(props),
            ..section.clone()
        };
        let count = crate::tables::table_paragraphs(table);
        let labels: Vec<Option<String>> = (t.block..t.block + count)
            .map(|b| src.labels.get(b).cloned().flatten())
            .collect();
        let mut work = self.work.borrow_mut();
        let (model, measurer) = &mut *work;
        let mut cx = TableContext {
            document: src.document,
            model,
            measurer,
            faces: src.faces,
            locale: src.locale,
        };
        let job = TableJob {
            table,
            section: &geometry,
            previous: previous.as_deref(),
            labels: &labels,
        };
        let flow = src.tables.flow_table(&mut cx, &job).ok()?;
        Some(TableRoom { left, width, flow })
    }
}

impl TableRooms for Rooms<'_> {
    fn room(&self, item: usize, left: i64, width: i64) -> Option<TableRoom> {
        if let Some(found) = self.found.borrow().get(&(item, left, width)) {
            return found.clone();
        }
        let found = self.lay_out(item, left, width);
        self.found.borrow_mut().insert((item, left, width), found.clone());
        found
    }
}
