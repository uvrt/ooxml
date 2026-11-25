//! Owns: a `w:p` as a [`Paragraph`] -- its runs and bookmarks, its properties, its table
//! context, its path and id, whether its mark is deleted, the section it ends.
//!
//! Must not: join paragraphs (`join`), or read runs (`runs`).

use super::fields::Fields;
use super::{Reader, Steps};
use crate::model::Paragraph;
use crate::value::{text, Text};
use crate::w;
use crate::xmlread::child;
use ooxml_xml::Element;
use std::sync::Arc;

/// Whether a paragraph's mark is deleted, or moved away, in the final view.
pub(crate) fn mark_removed(element: &Element) -> bool {
    child(element, "pPr")
        .and_then(|p| child(p, "rPr"))
        .is_some_and(|m| child(m, "del").is_some() || child(m, "moveFrom").is_some())
}

impl Reader<'_> {
    /// The paragraph `element`, in a table with `table_style` and `conditions` where it is
    /// in a cell.
    pub(crate) fn paragraph(
        &self,
        element: &Element,
        table: Option<(&Text, &Arc<[Text]>)>,
        path: Text,
        fields: &mut Fields,
    ) -> Paragraph {
        let mut runs = Vec::new();
        let outer = fields.bookmarks.replace(Vec::new());
        self.inline(element, &mut Steps::new(""), fields, &mut runs);
        let bookmarks = std::mem::replace(&mut fields.bookmarks, outer).unwrap_or_default();
        let ppr = child(element, "pPr");
        let section = ppr
            .and_then(|p| child(p, "sectPr"))
            .map(|s| Arc::new(super::section_props(s)));
        Paragraph {
            runs: Arc::from(runs),
            properties: self.para_props(ppr),
            table_style_id: table.map(|(s, _)| s.clone()),
            table_conditions: table.map_or_else(|| Arc::from(Vec::new()), |(_, c)| Arc::clone(c)),
            path,
            key: None,
            para_id: w::P::new(element).para_id().map(text),
            joins: Arc::from(Vec::new()),
            mark_deleted: mark_removed(element),
            section,
            bookmarks: Arc::from(bookmarks),
        }
    }
}
