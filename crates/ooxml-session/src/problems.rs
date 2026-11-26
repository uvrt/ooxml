//! Owns: the one shape of a document's problems in every reply (user decision 2026-11-25,
//! structured problems; R142): [`Problem`] -- `{code, part, element, message, severity}` --
//! and [`Validation`], a version's problems against those its document opened with: the
//! problems it added and those it fixed, and how many it opened with. An edit's
//! `checks.validate`, `check`'s problems and a refused save all say them so.
//!
//! Must not: find problems (each format validates its own documents) or word a format's
//! problem (its format gives the message).

use serde_json::{Map, Value};

/// How bad a problem is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    /// Office refuses or repairs the file: the save gate holds it against a version.
    Problem,
    /// Office opens and repairs it the same way each time: the save gate lets it through.
    Warning,
}

impl Severity {
    /// Its name in a reply.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Problem => "problem",
            Severity::Warning => "warning",
        }
    }
}

/// One problem of a document's version.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Problem {
    /// What is wrong (`paraid-repeated`, `bookmark-unpaired`...).
    pub code: String,
    /// The part it is in.
    pub part: String,
    /// The element, value or place it concerns.
    pub element: String,
    /// The problem in words, as a reply said it before it had this shape
    /// (`[code] part: detail`).
    pub message: String,
    /// How bad it is.
    pub severity: Severity,
}

impl Problem {
    /// As a reply carries it.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("code".into(), Value::from(self.code.as_str()));
        out.insert("part".into(), Value::from(self.part.as_str()));
        out.insert("element".into(), Value::from(self.element.as_str()));
        out.insert("message".into(), Value::from(self.message.as_str()));
        out.insert("severity".into(), Value::from(self.severity.as_str()));
        Value::Object(out)
    }
}

/// A version's problems against those its document opened with.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Validation {
    /// Each problem now that the document did not open with, in order.
    pub new: Vec<Problem>,
    /// Each problem the document opened with that is gone now.
    pub fixed: Vec<Problem>,
    /// How many problems the document opened with.
    pub baseline: usize,
}

impl Validation {
    /// As a reply carries it: `{new, fixed, baseline}`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let list = |items: &[Problem]| Value::Array(items.iter().map(Problem::to_json).collect());
        let mut out = Map::new();
        out.insert("new".into(), list(&self.new));
        out.insert("fixed".into(), list(&self.fixed));
        out.insert("baseline".into(), Value::from(self.baseline));
        Value::Object(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_validation_says_each_problem_in_the_one_shape() {
        let problem = Problem {
            code: "bookmark-unpaired".into(),
            part: "word/document.xml".into(),
            element: "start 7".into(),
            message: "[bookmark-unpaired] word/document.xml: start 7".into(),
            severity: Severity::Problem,
        };
        let validation = Validation {
            new: vec![problem],
            fixed: Vec::new(),
            baseline: 2,
        };
        assert_eq!(
            validation.to_json(),
            serde_json::json!({"new": [{"code": "bookmark-unpaired", "part": "word/document.xml",
                "element": "start 7", "message": "[bookmark-unpaired] word/document.xml: start 7",
                "severity": "problem"}], "fixed": [], "baseline": 2})
        );
    }
}
