//! Owns: what the Word tools do with a strict Open XML document
//! (`ooxml_word_edit::WordDoc::strict`, found once at open): the tools that only lay it out
//! and paint it ([`RENDERED`]) read it through the namespace-exact views; every other tool,
//! reading or changing, is refused with one refusal ([`refused`]), whose way out is the
//! document saved as a Word Document (.docx) in Word. The reading tools and the edits read
//! and write transitional WordprocessingML only: on a strict document they would see an
//! empty document and write what Word refuses to open (measured, Word 16.106: an inserted
//! paragraph made the file one Word asks to repair).
//!
//! Must not: find whether a document is strict (its open does), or refuse a tool here (the
//! session does, before the tool runs).

use ooxml_tools::{ToolError, WayOut};
use serde_json::Value;

/// The tools that read a strict document: they only lay it out and paint it.
pub const RENDERED: &[&str] = &["render"];

/// The form refused, as `details.not_supported` names it.
pub const STRICT: &str = "a strict Open XML document";

/// The refusal of a tool on a strict Open XML document: what is supported is rendering it,
/// or any tool on it saved as a Word Document (.docx) in Word.
#[must_use]
pub fn refused() -> ToolError {
    let way_out = WayOut::one(Value::from(
        "save it as a Word Document (.docx) in Word first; render draws it as it is",
    ));
    ToolError::not_supported(STRICT, way_out)
}
