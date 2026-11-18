//! The object tools' cases of `word-edits` (`op: "objects"`): `word_format_table` (and the
//! other object tools as they are ported) called with JSON arguments on a document as
//! opened, reported as the structure tools' cases are, for
//! `python/word_edits_diff.py --objects` to compare with docx-agent's toolbox.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::structure_tools::{run, tool_case};
use ooxml_tools_word::{word_format_table, WordFormatTable};
use serde_json::Value;
use std::sync::Arc;

/// One object tool case on a freshly opened document.
///
/// # Errors
/// A message when the document cannot be opened, or the tool is not ported.
pub fn case(bytes: &Arc<[u8]>, case: &Value) -> Result<Value, String> {
    tool_case(bytes, case, |tool, arguments, editing| match tool {
        "word_format_table" => Some(run(arguments, |args: &WordFormatTable| {
            word_format_table(args, editing)
        })),
        _ => None,
    })
}
