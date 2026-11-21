//! Owns: [`Limits`], the host's limits on what one call answers: its reply's characters and
//! the items a listing shows per page (docx-agent's `Limits.max_result_chars` and
//! `Limits.max_list_items`). A host gives them per call; a tool reads them where it pages.
//!
//! Must not: cut a reply ([`crate::ToolResult::to_text`] does) or know any tool.

use crate::MAX_RESULT_CHARS;

/// What a call's reply may hold, as the host sets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The reply within so many characters, as the model is sent it.
    pub result_chars: usize,
    /// The items a listing shows per page.
    pub list_items: usize,
}

impl Limits {
    /// The defaults (24,000 characters, 50 items), as docx-agent's `Limits()`.
    pub const DEFAULT: Self = Self {
        result_chars: MAX_RESULT_CHARS,
        list_items: 50,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}
