//! Owns: the regular-expression find the edits harness hands a replace: a Python pattern
//! matched by the tools' translation ([`ooxml_tools::pyre::PyRegex`]), each match's
//! replacement its template expanded as Python's `match.expand` does.
//!
//! Must not: be used by a library: the tool layer makes its own (phase P2c).

use ooxml_tools::pyre::PyRegex;
use ooxml_word_edit::find::Pattern;

/// A Python pattern and its replacement template.
#[derive(Debug)]
pub struct Regex {
    regex: PyRegex,
    template: String,
}

impl Regex {
    /// `pattern` (`ignore_case`: `re.IGNORECASE`), replaced by `template`.
    ///
    /// # Errors
    /// Why the pattern is not matched here.
    pub fn new(pattern: &str, ignore_case: bool, template: &str) -> Result<Regex, String> {
        Ok(Regex {
            regex: PyRegex::new(pattern, ignore_case).map_err(|e| e.to_string())?,
            template: template.to_owned(),
        })
    }
}

impl Pattern for Regex {
    fn matches(&self, text: &str) -> Result<Vec<(usize, usize, String)>, String> {
        self.regex
            .find_expanded(text, &self.template)
            .map_err(|e| e.to_string())
    }
}
