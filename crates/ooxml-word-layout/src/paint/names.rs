//! Owns: naming a page's painted items at the edge (phase 3 identity design, section 2): the
//! `data-docx-path` an item is written under, from its anchor through the version's names
//! ([`VersionNames`]) -- a block's path from its story's index, a run's below its paragraph's
//! from the paragraph's projection (the line's runs), a floating drawing's below its run's --
//! each block named once per page.
//!
//! A text box's items are named below their drawing's name ([`BoxNames`]): the slot's name
//! and the item's place in the drawing's content. No story's index holds a text box's
//! blocks as the layout reads them: a VML box is read from `mc:Fallback`, which the index
//! skips, and the paths below a drawing follow the drawing reader's own steps (its
//! `wp:anchor[n]`, a group's members, `wps:txbx/w:txbxContent`). Keying them waits for one
//! `mc:` rule on both sides (phase 3 identity design, RD1).
//!
//! Beside the anchors the paint still carries the paths it was named by (until LK2): an
//! anchored item's name is the path (asserted in debug builds); an item with no anchor (a
//! text box's in a drawing that has none, the endnotes' continuation separator the layout
//! makes) is written under its path.
//!
//! Must not: lay out or paint, or key anything by a name.

use super::drawings::{anchor_below, DrawingSlot};
use super::PaintedLine;
use ooxml_word_model::docx::VersionNames;
use ooxml_word_model::ids::{Anchor, BlockKey, Sub};
use ooxml_word_model::model::Run;
use std::borrow::Cow;
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
            .filter_map(|line| Some((line.anchor?.block, &*line.runs)))
            .collect();
        PageNames {
            names,
            runs,
            blocks: RefCell::new(HashMap::new()),
        }
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

    /// The name an item drawn for `anchor` and painted with `path` is written under: its
    /// anchor's (the path, asserted in debug builds), else its path.
    pub fn written(&self, anchor: Option<Anchor>, path: &str) -> String {
        let Some(anchor) = anchor else {
            return path.to_owned();
        };
        let named = self.name(anchor);
        debug_assert_eq!(named.as_deref(), Some(path), "{anchor:?} is named as it was painted");
        named.unwrap_or_else(|| path.to_owned())
    }

    /// The names of the items of the text boxes in the drawing `slot`: below its name.
    pub fn boxed<'n>(&'n self, slot: &'n DrawingSlot) -> BoxNames<'n, 'a> {
        BoxNames {
            page: self,
            name: slot.anchor.map(|_| self.slot(slot)),
            path: &slot.path,
        }
    }

    /// The name a drawing's slot is written under: an inline one's run's, a floating one's
    /// its `wp:anchor[n]` below its run (a VML shape's own path).
    pub fn slot(&self, slot: &DrawingSlot) -> String {
        let Some(anchor) = slot.anchor.filter(|_| !slot.inline) else {
            return self.written(slot.anchor, &slot.path);
        };
        let named = (|| {
            let paragraph = self.block(Anchor::whole(anchor.block))?;
            let runs = self.runs.get(&anchor.block)?;
            Some(match anchor_below(runs, slot.run, slot.index) {
                Some(below) => format!("{paragraph}/{below}"),
                None => paragraph,
            })
        })();
        debug_assert_eq!(
            named.as_deref(),
            Some(&*slot.path),
            "{anchor:?}'s drawing is named as it was painted"
        );
        named.unwrap_or_else(|| slot.path.clone())
    }

    /// `slot` as its painter is handed it: named at the edge (its path and its paragraph's),
    /// as it was where it has no anchor.
    pub fn named_slot<'s>(&self, slot: &'s DrawingSlot) -> Cow<'s, DrawingSlot> {
        let Some(anchor) = slot.anchor else {
            return Cow::Borrowed(slot);
        };
        Cow::Owned(DrawingSlot {
            path: self.slot(slot),
            paragraph_path: self.written(Some(Anchor::whole(anchor.block)), &slot.paragraph_path),
            ..slot.clone()
        })
    }
}

/// What names an item at the edge: its anchor's name, else its path.
pub trait Namer {
    /// The name an item drawn for `anchor` and painted with `path` is written under.
    fn written(&self, anchor: Option<Anchor>, path: &str) -> String;
}

impl Namer for PageNames<'_> {
    fn written(&self, anchor: Option<Anchor>, path: &str) -> String {
        PageNames::written(self, anchor, path)
    }
}

/// The names of a text box's items: below the name of the drawing they are in (`None`: a
/// drawing with no anchor, its items written under their paths). Until LK2 an item's place
/// in the drawing's content is its path past the drawing's.
pub struct BoxNames<'n, 'a> {
    page: &'n PageNames<'a>,
    name: Option<String>,
    path: &'n str,
}

impl BoxNames<'_, '_> {
    /// The name of an item painted with `path`: the drawing's name and the item's place
    /// below it (the path, asserted in debug builds).
    fn below(&self, path: &str) -> Option<String> {
        let name = self.name.as_deref()?;
        let rest = path.strip_prefix(self.path).filter(|rest| rest.starts_with('/'))?;
        let named = format!("{name}{rest}");
        debug_assert_eq!(
            named, path,
            "a text box's item is named below its drawing as it was painted"
        );
        Some(named)
    }

    /// `slot`, a drawing inline in a text box, as its painter is handed it: named below the
    /// text box's drawing.
    pub fn named_slot<'s>(&self, slot: &'s DrawingSlot) -> Cow<'s, DrawingSlot> {
        if slot.anchor.is_some() {
            return self.page.named_slot(slot);
        }
        match (self.below(&slot.path), self.below(&slot.paragraph_path)) {
            (Some(path), Some(paragraph_path)) => Cow::Owned(DrawingSlot {
                path,
                paragraph_path,
                ..slot.clone()
            }),
            _ => Cow::Borrowed(slot),
        }
    }
}

impl Namer for BoxNames<'_, '_> {
    fn written(&self, anchor: Option<Anchor>, path: &str) -> String {
        if anchor.is_some() {
            return self.page.written(anchor, path);
        }
        self.below(path).unwrap_or_else(|| path.to_owned())
    }
}
