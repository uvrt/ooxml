//! Another editor's change in the way of a call (co-authoring, phase 3 CF2): one refusal,
//! said one way ([`ToolError::changed_meanwhile`]).
//!
//! Owns: the wording of the refusal of an edit or batch prepared before another editor's
//! version came in that changed what it read, and of an undo or redo entangled with such a
//! version: who changed what, in `details.changed_by` and `details.changed`, with the way
//! out (read them again and decide; an undo never takes back another author's change).
//!
//! Must not: know any format or tool; the session names the editor and the blocks.

use crate::error::{Refusal, ToolError, WayOut};
use serde_json::Value;

/// What a call met that another editor changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meanwhile {
    /// An edit or batch prepared before the change: not run again over it.
    Edit,
    /// An undo or redo whose step that change is entangled with.
    History,
}

impl ToolError {
    /// The refusal of a call another editor's version (`editor`'s, the document's version
    /// `version`) is in the way of: it changed `changed` (block ids, part names).
    #[must_use]
    pub fn changed_meanwhile(meanwhile: Meanwhile, editor: &str, version: u64, changed: &[String]) -> Self {
        let what = if changed.is_empty() {
            "what this call depends on".to_owned()
        } else {
            changed.join(", ")
        };
        let (message, way_out) = match meanwhile {
            Meanwhile::Edit => (
                format!("{editor} changed {what} (version {version}) since this call read it; nothing changed here"),
                "read them again, then send the call again if it still applies",
            ),
            Meanwhile::History => (
                format!("{editor}'s change (version {version}) is in the way: it changed {what}; undo does not take back another author's change"),
                "edit those blocks directly to change them back",
            ),
        };
        Self::refusal(Refusal::Refused, message, WayOut::one(Value::from(way_out)))
            .with_detail("changed_by", Value::from(editor))
            .with_detail("changed", Value::from(changed.to_vec()))
    }
}
