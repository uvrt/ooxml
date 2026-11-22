//! Owns: what a scale read of a document needs of the scale (`ooxml_edit.tools.scales.ReadScale`,
//! as the host's tool layer parsed it): its name, its steps' colours and labels, its targets
//! and whether its texts show labels; the step a style's name is for (`step_of_style`), the
//! step a label names (`step_of_label`) and a step's style's name (`style_name`).
//!
//! Must not: parse a scale's argument or choose a value's step (the host does).

use crate::written::py_space;
use ooxml_word_model::docx::casefold;

/// One step of a scale, as a read of values needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadStep {
    /// Its fill, `RRGGBB` upper case.
    pub fill: String,
    /// Its text colour, `RRGGBB` upper case.
    pub text: String,
    /// Its label, where it has one.
    pub label: Option<String>,
}

/// A scale, as a read of values needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadScale {
    /// Its name (`Score`): its steps' styles are named from it.
    pub name: String,
    /// Its steps, in order.
    pub steps: Vec<ReadStep>,
    /// Its targets, as given.
    pub targets: Vec<String>,
    /// Whether its texts show its steps' labels (`"show": "label"`).
    pub labels: bool,
}

/// Python's `" ".join(text.split()).casefold()` (`_folded`).
fn folded(text: &str) -> String {
    casefold(
        &text
            .split(py_space)
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Python's `str.strip()`.
pub(crate) fn py_strip(text: &str) -> &str {
    text.trim_matches(py_space)
}

impl ReadScale {
    /// The index of the step whose label `text` is (case and spaces aside), or `None`.
    #[must_use]
    pub fn step_of_label(&self, text: &str) -> Option<usize> {
        let wanted = folded(text);
        self.steps
            .iter()
            .position(|s| s.label.as_deref().is_some_and(|l| !l.is_empty() && folded(l) == wanted))
    }

    /// The name of the step `index`'s style: the scale's name, the step's number, its label.
    #[must_use]
    pub fn style_name(&self, index: usize) -> String {
        let label = self
            .steps
            .get(index)
            .and_then(|s| s.label.as_deref())
            .filter(|l| !l.is_empty());
        match label {
            Some(label) => format!("{} {} {label}", self.name, index + 1),
            None => format!("{} {}", self.name, index + 1),
        }
    }

    /// The index of the step a style named `name` is for -- this scale's name and a step's
    /// number, whatever follows after a space, case aside (`step_of_style`) -- or `None`.
    #[must_use]
    pub fn step_of_style(&self, name: Option<&str>) -> Option<usize> {
        let name = py_strip(name.filter(|n| !n.is_empty())?);
        let rest = strip_prefix_caseless(name, &self.name)?.strip_prefix(' ')?;
        let digits: &str = &rest[..rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len())];
        let after = &rest[digits.len()..];
        if digits.is_empty() || !(after.is_empty() || (after.starts_with(' ') && !after.contains('\n'))) {
            return None;
        }
        let number: usize = digits.parse().ok()?;
        (1..=self.steps.len()).contains(&number).then(|| number - 1)
    }
}

/// `text` without `prefix` at its start, compared as `re.IGNORECASE` compares them.
fn strip_prefix_caseless<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let mut chars = text.char_indices();
    for wanted in prefix.chars() {
        let (_, have) = chars.next()?;
        if have != wanted && !have.to_lowercase().eq(wanted.to_lowercase()) {
            return None;
        }
    }
    Some(chars.next().map_or("", |(at, _)| &text[at..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale() -> ReadScale {
        let step = |label: Option<&str>| ReadStep {
            fill: "FF0000".into(),
            text: "000000".into(),
            label: label.map(str::to_owned),
        };
        ReadScale {
            name: "Score".into(),
            steps: vec![step(Some("Low")), step(None), step(Some("High"))],
            targets: vec![],
            labels: true,
        }
    }

    #[test]
    fn a_style_is_a_step_s_by_the_scale_s_name_and_its_number() {
        let s = scale();
        assert_eq!(s.step_of_style(Some("Score 1 Low")), Some(0));
        assert_eq!(s.step_of_style(Some(" score 3")), Some(2));
        assert_eq!(s.step_of_style(Some("Score 3 Old label")), Some(2));
        assert_eq!(s.step_of_style(Some("Score 4")), None);
        assert_eq!(s.step_of_style(Some("Score 0")), None);
        assert_eq!(s.step_of_style(Some("Score1")), None);
        assert_eq!(s.step_of_style(Some("Scores 1")), None);
        assert_eq!(s.step_of_style(None), None);
        assert_eq!(s.style_name(0), "Score 1 Low");
        assert_eq!(s.style_name(1), "Score 2");
    }

    #[test]
    fn a_label_is_found_case_and_spaces_aside() {
        let s = scale();
        assert_eq!(s.step_of_label("  high "), Some(2));
        assert_eq!(s.step_of_label("Medium"), None);
    }
}
