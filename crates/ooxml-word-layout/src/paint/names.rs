//! Owns: naming a page's painted items at the edge (phase 3 identity design, section 2): the
//! `data-docx-path` an item is written under, from its anchor through the version's names
//! ([`VersionNames`]) -- a block's path from its story's index, a run's below its paragraph's
//! from the paragraph's projection (the line's runs), a floating drawing's below its run's --
//! each block named once per page.
//!
//! A text box's blocks are keyed by their own keys (phase 3 identity design, RD1: the index
//! keys the branch of an `mc:AlternateContent` the layout reads) and named like any block:
//! the version's names write the drawing reader's steps below their paragraph (its
//! `wp:anchor[n]`, a group's members, `wps:txbx/w:txbxContent`).
//!
//! An item no story's index holds is named by the path it was made with ([`ItemName::Path`]):
//! a paragraph the layout makes (the endnotes' continuation separator), a block of a part no
//! index holds. Everything else carries only its anchor.
//!
//! Must not: lay out or paint, or key anything by a name.

use super::drawings::{anchor_below, DrawingSlot, SlotName, SlotPaths};
use super::{PagePaint, PaintedLine};
pub use crate::naming::ItemName;
use ooxml_word_model::docx::VersionNames;
use ooxml_word_model::ids::{Anchor, BlockKey, Sub};
use ooxml_word_model::model::Run;
use std::cell::RefCell;
use std::collections::HashMap;

/// A page's items' names: the version's names, the runs of the paragraphs the page draws
/// lines of, and each block named so far.
pub struct PageNames<'a> {
    names: &'a VersionNames,
    runs: HashMap<BlockKey, &'a [Run]>,
    blocks: RefCell<HashMap<Anchor, Option<String>>>,
}

impl<'a> PageNames<'a> {
    /// The names of a page of the version named by `names` whose lines are `lines`.
    pub fn new(names: &'a VersionNames, lines: impl IntoIterator<Item = &'a PaintedLine>) -> PageNames<'a> {
        let runs = lines
            .into_iter()
            .filter_map(|line| Some((line.name.anchor()?.block, &*line.runs)))
            .collect();
        PageNames {
            names,
            runs,
            blocks: RefCell::new(HashMap::new()),
        }
    }

    /// The names of page `paint` of the version named by `names`: its lines' and its text
    /// boxes'.
    pub fn of_page(names: &'a VersionNames, paint: &'a PagePaint) -> PageNames<'a> {
        let text_lines = paint.text_boxes.iter().flat_map(|t| t.lines.iter());
        PageNames::new(names, paint.lines.iter().chain(text_lines))
    }

    /// The name of `anchor`, a block or a place on it the index names, once per page.
    fn block(&self, anchor: Anchor) -> Option<String> {
        if let Some(found) = self.blocks.borrow().get(&anchor) {
            return found.clone();
        }
        let named = self.names.path(anchor);
        self.blocks.borrow_mut().insert(anchor, named.clone());
        named
    }

    /// `anchor`'s name: a run's is its paragraph's and the run's path in the paragraph's
    /// projection (the paragraph's where the run has none).
    pub fn name(&self, anchor: Anchor) -> Option<String> {
        let Sub::Run(k) = anchor.sub else {
            return self.block(anchor);
        };
        let paragraph = self.block(Anchor::whole(anchor.block))?;
        let run = self.runs.get(&anchor.block)?.get(usize::try_from(k).ok()?)?;
        Some(if run.path.is_empty() {
            paragraph
        } else {
            format!("{paragraph}/{}", run.path)
        })
    }

    /// The name an item named `name` is written under (empty where its block is not in
    /// the version).
    pub fn written(&self, name: &ItemName) -> String {
        match name {
            ItemName::Anchored(anchor) => self.name(*anchor).unwrap_or_default(),
            ItemName::Path(path) => path.to_string(),
        }
    }

    /// The name a drawing's slot is written under: an inline one's run's, a floating one's
    /// its `wp:anchor[n]` below its run (a VML shape's own path).
    pub fn slot(&self, slot: &DrawingSlot) -> String {
        let anchor = match &slot.name {
            SlotName::Anchored(anchor) => *anchor,
            SlotName::Path { path, .. } => return path.to_string(),
        };
        if slot.inline {
            return self.name(anchor).unwrap_or_default();
        }
        (|| {
            let paragraph = self.block(Anchor::whole(anchor.block))?;
            let runs = self.runs.get(&anchor.block)?;
            Some(match anchor_below(runs, slot.run, slot.index) {
                Some(below) => format!("{paragraph}/{below}"),
                None => paragraph,
            })
        })()
        .unwrap_or_default()
    }

    /// `slot`'s paths as its painter is handed them: its own and its paragraph's.
    pub fn slot_paths(&self, slot: &DrawingSlot) -> SlotPaths {
        match &slot.name {
            SlotName::Anchored(anchor) => SlotPaths {
                path: self.slot(slot),
                paragraph: self.block(Anchor::whole(anchor.block)).unwrap_or_default(),
            },
            SlotName::Path { path, paragraph } => SlotPaths {
                path: path.to_string(),
                paragraph: paragraph.to_string(),
            },
        }
    }
}

/// What names an item at the edge.
pub trait Namer {
    /// The name an item named `name` is written under.
    fn written(&self, name: &ItemName) -> String;
}

impl Namer for PageNames<'_> {
    fn written(&self, name: &ItemName) -> String {
        PageNames::written(self, name)
    }
}
