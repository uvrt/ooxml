//! The style tools' cases of `word-edits` (`op: "style"`): `word_lists`, `word_format` (and the other
//! style tools as they are ported) called with JSON arguments on a document as opened,
//! compared as the structure tools' cases are (`structure_tools::tool_case`), for
//! `python/word_edits_diff.py --style` to compare with docx-agent's toolbox.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::structure_tools::{run, tool_case};
use ooxml_tools_word::{word_format, word_lists, WordFormat, WordLists};
use serde_json::Value;
use std::sync::Arc;

/// One style tool case on a freshly opened document.
///
/// # Errors
/// A message when the document cannot be opened.
pub fn case(bytes: &Arc<[u8]>, case: &Value) -> Result<Value, String> {
    tool_case(bytes, case, |tool, arguments, editing| match tool {
        "word_lists" => Some(run(arguments, |args: &WordLists| word_lists(args, editing))),
        "word_format" => Some(run(arguments, |args: &WordFormat| word_format(args, editing))),
        _ => None,
    })
}
