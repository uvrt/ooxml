//! Owns: [`Bringer`], what a copy brings for each piece of copied content -- its styles,
//! the lists it numbers with, the relationships it names and the parts they reach, and
//! (for the blocks and their notes) its bookmarks and content-control ids -- as
//! docx-agent's importer runs `_formatting`, `_lists`, `_relationships`, `_bookmarks_in`
//! and `_control_ids` on the blocks, then on each note and each comment they hold, with
//! one set of maps for the whole copy and one list of warnings in the order they arise.
//!
//! Must not: place the content; it works on copies the caller holds.

use super::bookmarks::Bookmarks;
use super::parts::Parts;
use super::styles::Styles;
use crate::imported::Controls;
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::Result;

/// The styles, lists and parts one copy brings, and the copies of the parts they change.
pub(super) struct Bringer<'s> {
    /// The style and list maps.
    pub(super) styles: Styles<'s>,
    /// The copies of the styles and numbering parts.
    pub(super) space: Space,
    /// The relationships and parts.
    pub(super) parts: Parts,
    /// The bookmarks' names and renames.
    pub(super) bookmarks: Bookmarks,
    /// The content-control ids taken.
    pub(super) controls: Controls,
}

impl Bringer<'_> {
    /// `elements` (copied from `source_part`, held in `work`) made to fit `dest_part`: their
    /// styles, then their lists, then their relationships (the parts these reach put in
    /// place now, so that what the copy makes next is made after them).
    ///
    /// # Errors
    /// What the styles, lists or parts refuse or leave to the host.
    pub(super) fn content(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        work: &mut Work,
        elements: &[Id],
        parts: (&str, &str),
    ) -> Result<()> {
        for &element in elements {
            self.styles.bring(sink, &mut self.space, work, element)?;
        }
        for &element in elements {
            self.styles.bring_lists(sink, &mut self.space, work, element)?;
        }
        self.parts.bring(sink, work, elements, parts)
    }

    /// The bookmarks of `elements` (held in `work`) brought, their warnings after the
    /// warnings so far.
    ///
    /// # Errors
    /// When the document cannot be read or an annotation id given.
    pub(super) fn bookmarks(&mut self, sink: &mut EditSink<'_, '_>, work: &mut Work, elements: &[Id]) -> Result<()> {
        let warnings = self.bookmarks.bring(sink, work, elements)?;
        self.styles.warnings.extend(warnings);
        Ok(())
    }

    /// The content controls of `elements` (held in `work`) given ids no control has; each
    /// id changed (`sdt#old`, `sdt#new`), in order.
    ///
    /// # Errors
    /// When the document cannot be read.
    pub(super) fn controls(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        work: &mut Work,
        elements: &[Id],
    ) -> Result<Vec<(String, String)>> {
        let changed = self.controls.renumber(sink, work, elements)?;
        Ok(changed
            .into_iter()
            .map(|(old, new)| (format!("sdt#{old}"), format!("sdt#{new}")))
            .collect())
    }

    /// The styles and numbering parts put back; the warnings (the styles imported, the bookmarks renamed).
    ///
    /// # Errors
    /// When a part cannot be put.
    pub(super) fn finish(self, sink: &mut EditSink<'_, '_>) -> Result<Vec<String>> {
        self.space.finish(sink)?;
        Ok(self.styles.warnings)
    }
}
