//! Owns: every data-bound control's custom XML node set to what the control shows
//! (docx-agent's `_sync_bound_controls`) -- after an edit that changed what a control shows
//! without filling it (a review): Word fills a bound control from its node on opening.
//! The value is a check box's `true`/`false`, a date's `w:fullDate` (its `Z` dropped; its
//! text without one), a list control's item value for the text it shows (else that
//! text), any other control's text; a node holding elements is left as it is.
//!
//! Must not: make a node or part a binding names but the document lacks (such a control
//! is left, as docx-agent leaves it), or write but through the sink.

use super::bound::Binding;
use crate::names::W;
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_word_model::docx::controls::{controls, property, Control};
use ooxml_word_model::docx::StoryPart;

/// Every bound control's node written with the value the control shows, where it differs.
///
/// # Errors
/// [`EditError::Unsupported`] for a binding whose XPath is read only by docx-agent (the
/// host syncs it); what the sink refuses.
pub(crate) fn sync_bound_controls(sink: &mut EditSink<'_, '_>) -> Result<()> {
    let docx = sink.docx()?;
    for control in controls(&docx) {
        let Some(binding) = Binding::of(&control.element) else {
            continue;
        };
        let Some(part) = docx.part(&control.part) else {
            continue;
        };
        let value = value_of(&control, part);
        match binding.sync(sink, &docx, &value) {
            Ok(()) | Err(EditError::Refused(_) | EditError::NoPart(_)) => {}
            Err(other) => return Err(other),
        }
    }
    Ok(())
}

/// What a bound control's node holds for what the control shows now.
fn value_of(control: &Control, part: &StoryPart) -> String {
    match control.kind() {
        "checkbox" => if control.checked() == Some(true) {
            "true"
        } else {
            "false"
        }
        .to_owned(),
        "date" => property(&control.element, W, "date")
            .and_then(|d| d.attribute(Some(W), "fullDate"))
            .filter(|full| !full.is_empty())
            .map_or_else(|| control.text(part), |full| full.replace('Z', "")),
        "drop-down" | "combo-box" => {
            let text = control.text(part);
            control
                .items()
                .into_iter()
                .find(|(display, _)| *display == text)
                .map_or(text, |(_, value)| value)
        }
        _ => control.text(part),
    }
}
