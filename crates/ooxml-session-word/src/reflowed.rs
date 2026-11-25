//! Owns: the reflow a changing call's checks report, made after the call's edits
//! ([`crate::WordMade::reflow`]): from the version its first edit was
//! applied to, where the document's last check saw it or its engine (the host's, else the
//! session's own) laid it out last, to the version its last edit made
//! (`ooxml_tools_word::reflow_after_edit`).
//!
//! Must not: lay out but through the call's layout (its engine, or a spare made from its
//! seed where another call holds the engine), or change what the last check saw.

use crate::word::Word;
use ooxml_base::Cancel;
use ooxml_tools_word::{reflow_after_edit, EditedFrom, LayoutLimit, Made, WordLayout};
use serde_json::Value;

/// The reflow of the call whose edits made `made` in `word`, laid out by `layout` under
/// the call's limit on laying out (else until `cancel`); `None` where none is measured (not
/// a Word document, or no layout of the version before).
pub(crate) fn after_edit(
    layout: &dyn WordLayout,
    word: &Word,
    made: &Made,
    stop: (Option<LayoutLimit<'_>>, &dyn Cancel),
) -> Option<Value> {
    let before = &made.before;
    let from = EditedFrom {
        version: before.seq().0,
        package: before.doc().package(),
    };
    let after = made.last.doc();
    reflow_after_edit(
        layout,
        &word.last_check,
        from,
        after.package(),
        after.ids().aliases(),
        stop,
    )
}
