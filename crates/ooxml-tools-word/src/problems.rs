//! Owns: a Word document's validation problems in the session's one shape
//! ([`ooxml_session::problems`], user decision 2026-11-25): each problem's code and part, the
//! element it concerns (the edit core's detail), today's words for it, and its severity (a
//! warning for what Word repairs the same way each time, [`ooxml_word_edit::validate::WARNINGS`]).
//!
//! Each problem of a list as a reply says it ([`listed`]): `check`'s and the edits' alike.
//!
//! Must not: find problems (the edit core validates).

use ooxml_tools::problems::{Problem, Severity, Validation};
use ooxml_word_edit::validate::{Delta, Problem as Found};

/// `found` in the session's shape.
#[must_use]
pub fn problem(found: &Found) -> Problem {
    Problem {
        code: found.code.clone(),
        part: found.part.clone(),
        element: found.detail.clone(),
        message: found.to_string(),
        severity: if found.is_warning() {
            Severity::Warning
        } else {
            Severity::Problem
        },
    }
}

/// `delta` in the session's shape.
#[must_use]
pub fn structured(delta: &Delta) -> Validation {
    Validation {
        new: delta.new_found.iter().map(problem).collect(),
        fixed: delta.fixed_found.iter().map(problem).collect(),
        baseline: delta.baseline,
    }
}

/// `found` as a reply lists them: each in the one shape, as JSON.
#[must_use]
pub fn listed<'a>(found: impl IntoIterator<Item = &'a Found>) -> serde_json::Value {
    serde_json::Value::Array(found.into_iter().map(|p| problem(p).to_json()).collect())
}
