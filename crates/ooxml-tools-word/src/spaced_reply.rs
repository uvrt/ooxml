//! Owns: what a reply says when the text its edits wrote got its language's spacing
//! ([`note_spaced`]; docx-agent's `SPACED`, round 35 R21): how many spaces before : ; ! ?
//! or inside « » were written as no-break spaces.
//!
//! Must not: space anything (the edit's sink does, `ooxml_word_edit::Change::Spaced`
//! counts it) or say anything of a call that failed or spaced nothing.

use ooxml_tools::{Outcome, ToolResult};
use serde_json::Value;

/// `result` with the warning that `spaced` spaces were written as no-break spaces, where
/// it is done and spaced some.
pub fn note_spaced(result: &mut ToolResult<Value>, spaced: usize) {
    if spaced == 0 || !matches!(result.outcome, Outcome::Done(_)) {
        return;
    }
    result.warnings.push(format!(
        "{spaced} space(s) before : ; ! ? or inside « » written as no-break spaces (U+00A0), as Word's \
         proofing of the text's language expects"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_tools::{Done, ToolError};

    #[test]
    fn a_done_call_says_how_many() {
        let mut result = ToolResult::<Value>::done(Done::new("ok"));
        note_spaced(&mut result, 2);
        assert_eq!(
            result.warnings,
            [
                "2 space(s) before : ; ! ? or inside « » written as no-break spaces (U+00A0), as Word's proofing of \
              the text's language expects"
            ]
        );
    }

    #[test]
    fn nothing_spaced_or_failed_says_nothing() {
        let mut result = ToolResult::<Value>::done(Done::new("ok"));
        note_spaced(&mut result, 0);
        assert!(result.warnings.is_empty());
        let mut failed = ToolResult::<Value>::failed(ToolError::internal("x", "y"));
        note_spaced(&mut failed, 3);
        assert!(failed.warnings.is_empty());
    }
}
