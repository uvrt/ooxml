//! Owns: what docx-agent's `_edit` does after every edit before it ends: the written text's
//! spacing (`_space_written`, [`crate::spacing`]), text boxes'
//! fallbacks synced with what changed in them (`_sync_text_boxes`) and revision ids made
//! unique again (`_unique_revision_ids`).
//!
//! Must not: write but through the sink.

use crate::sink::EditSink;
use crate::Result;

/// The steps after an edit, through `sink`.
///
/// # Errors
/// When a part is not XML.
pub(crate) fn after_edit(sink: &mut EditSink<'_, '_>) -> Result<()> {
    sink.space_written()?;
    crate::textbox::sync(sink)?;
    crate::revision_ids::make_unique(sink)
}
