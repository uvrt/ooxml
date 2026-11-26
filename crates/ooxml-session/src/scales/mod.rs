//! Values styled by a scale given once: `check`'s `scale` (design phase 3, section 5;
//! ooxml-edit's `scales`, its check ported exactly), a session service for every format.
//!
//! Owns: a scale read or refused ([`parse`]); a value's step from its text -- one number in
//! the locale, or a step's text -- or from the label its text shows and the record kept with
//! it, its source read again where the session can ([`read`]); and the check's fact
//! ([`report`]): the values styled with another step than their value's (`wrong`), those with
//! none (`none`), for a scale of labels the texts that are not their value's label (`labels`)
//! and the labels with no value recorded (`unrecorded`), capped, with the one
//! `style_by_value` call that fixes them.
//!
//! Must not: read a document (its format reads the targets' values and the steps they are
//! styled with now), or style one (each format's `style_by_value`).

mod contrast;
mod parse;
mod report;

pub use contrast::{hex_colour, ratio, readable_text, BLACK, TEXT_MINIMUM, WHITE};
pub use parse::{example, parse, shows_labels};
pub use report::{report, summary, ScaleEnv, ScaleValue};

use ooxml_numfmt::Locale;
use serde_json::{Map, Value};

/// The tool that applies a scale.
pub const TOOL: &str = "style_by_value";
/// `check`'s parameter that asks for the fact.
pub const PARAM: &str = "scale";
/// Steps in one scale at most.
pub const MAX_STEPS: usize = 12;
/// Targets in one scale at most (a table column or a reference counts once).
pub const MAX_TARGETS: usize = 50;
/// Values a check lists (wrong and none together).
pub const CHECK_LISTED: usize = 20;
/// Skipped values a check lists.
pub const SKIPPED_LISTED: usize = 10;
/// The name a scale's styles take when it gives none.
pub const DEFAULT_NAME: &str = "Step";

/// How a scale's steps take values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    /// Each step a `min` (and `max`).
    Range,
    /// Each step one number.
    Value,
    /// Each step one text.
    Text,
}

/// One step: its place (from 0), its fill and text colours `RRGGBB` (`rule` says how the
/// text was chosen), the WCAG 2 contrast between them, its label, and what it takes.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// Its place, from 0.
    pub index: usize,
    /// Its fill.
    pub fill: String,
    /// Its text colour.
    pub text: String,
    /// `white`, `black` or `auto`.
    pub rule: String,
    /// The contrast of its colours, to two places.
    pub contrast: f64,
    /// Its label.
    pub label: Option<String>,
    /// The least value it takes.
    pub min: Option<f64>,
    /// The most.
    pub max: Option<f64>,
    /// The one value it takes, as given.
    pub value: Option<String>,
    /// That value, where it is a number.
    pub number: Option<f64>,
}

impl Step {
    /// Its number, from 1, as styles are named.
    #[must_use]
    pub fn n(&self) -> usize {
        self.index + 1
    }
}

/// A scale as [`parse`] read it: `given` as the caller gave it (for the fix a check offers),
/// `notes` what the reading has to say, `labels` whether its texts show its steps' labels.
#[derive(Debug, Clone, PartialEq)]
pub struct Scale {
    /// Its name.
    pub name: String,
    /// Its steps, lowest first.
    pub steps: Vec<Step>,
    /// Its targets.
    pub targets: Vec<String>,
    /// How its steps take values.
    pub by: By,
    /// The argument as given.
    pub given: Map<String, Value>,
    /// What the reading has to say.
    pub notes: Vec<String>,
    /// Whether its texts show its steps' labels.
    pub labels: bool,
}

/// Python's `" ".join(str(text).split()).casefold()` (`_folded`).
pub(crate) fn folded(text: &str) -> String {
    crate::numbers::casefold(
        &text
            .split(crate::facts::is_py_space)
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// A scale refused for a document of `kind` whose format reads none (`values_of`), naming
/// the kinds that do (`reading`).
#[must_use]
pub fn no_reading(reading: &[&str], kind: &str, way_out: ooxml_tools::WayOut) -> ooxml_tools::ToolError {
    let listed = if reading.is_empty() {
        "no".to_owned()
    } else {
        reading.join(" and ")
    };
    ooxml_tools::ToolError::refusal(
        ooxml_tools::Refusal::InvalidArguments,
        format!("a scale styles values in {listed} documents here, not a {kind}"),
        way_out,
    )
    .at(PARAM)
}

/// A number as a scale's words write it (`_num`): `{:g}` for a whole one or one of at least
/// `1e-4`, else its `repr`.
#[must_use]
pub fn num(value: f64) -> String {
    if value.fract() == 0.0 || value.abs() >= 1e-4 {
        ooxml_base::pyfmt::general(value, 6)
    } else {
        ooxml_base::pyfmt::repr_float(value)
    }
}

/// `text` shortened for a message, Python's `repr` (`_quote`).
fn quoted(text: &str) -> String {
    let joined = text
        .split(crate::facts::is_py_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let shown = if joined.chars().count() <= 40 {
        joined
    } else {
        let mut cut: String = joined.chars().take(39).collect();
        cut.push('…');
        cut
    };
    ooxml_base::pyfmt::repr_str(&shown)
}

/// The patterns a value is read with: a score's scale at its end (`_OUT_OF`) and a percent
/// sign (`_PERCENT`).
static PATTERNS: std::sync::LazyLock<Result<(fancy_regex::Regex, fancy_regex::Regex), String>> =
    std::sync::LazyLock::new(|| {
        Ok((
            crate::numbers::compiled(r"\s*(?:/|out\s+of)\s*\d+(?:[.,]\d+)?\s*$", true)?,
            crate::numbers::compiled(r"\s*[%‰]\s*$", false)?,
        ))
    });

/// `text` without the first match of `pattern` (`pattern.sub("", text, 1)`).
fn without(pattern: &fancy_regex::Regex, text: &str) -> String {
    match pattern.find(text).ok().flatten() {
        Some(found) => format!(
            "{}{}",
            text.get(..found.start()).unwrap_or_default(),
            text.get(found.end()..).unwrap_or_default()
        ),
        None => text.to_owned(),
    }
}

/// `text` read as one number in `locale` (`read_value`): `2,5` or `2.5`, thousands
/// separators, a sign, a percentage as written (`85 %` is 85), `3/5` and `3 out of 5` 3.
#[must_use]
pub fn read_value(text: &str, locale: &Locale) -> Option<f64> {
    let (out_of, percent) = PATTERNS.as_ref().ok()?;
    let spaced = text.replace(['\u{a0}', '\u{202f}'], " ");
    let mut body = spaced
        .split(crate::facts::is_py_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    body = without(percent, &without(out_of, &body));
    let mut sign = 1.0;
    if let Some(rest) = body.strip_prefix(['-', '\u{2212}', '\u{2012}', '\u{2013}']) {
        sign = -1.0;
        body = rest.trim_start_matches(crate::facts::is_py_space).to_owned();
    } else if let Some(rest) = body.strip_prefix('+') {
        body = rest.trim_start_matches(crate::facts::is_py_space).to_owned();
    }
    if !body.chars().next().is_some_and(ooxml_base::pystr::is_digit) {
        let rest = body.get(1..).unwrap_or_default();
        if body.starts_with(['.', ',']) && ooxml_base::pystr::str_is_digit(rest) {
            body = format!("0{body}");
        } else {
            return None;
        }
    }
    let group = locale.thousands_separator;
    if [' ', '\u{a0}', '\u{202f}'].contains(&group) {
        body = body.replace(' ', &group.to_string());
    }
    if !digits_and_separators(&body) {
        return None;
    }
    let (value, ..) = crate::numbers::parse(&body, locale)?;
    Some(sign * value.to_f64())
}

/// `re.fullmatch(r"\d+(?:[.,'’ \xa0\u202f]\d+)*", body)`.
fn digits_and_separators(body: &str) -> bool {
    let separator = |c: char| matches!(c, '.' | ',' | '\'' | '\u{2019}' | ' ' | '\u{a0}' | '\u{202f}');
    let mut last_digit = false;
    for c in body.chars() {
        if ooxml_base::pystr::is_decimal(c) {
            last_digit = true;
        } else if separator(c) && last_digit {
            last_digit = false;
        } else {
            return false;
        }
    }
    last_digit
}

#[cfg(test)]
mod tests;
