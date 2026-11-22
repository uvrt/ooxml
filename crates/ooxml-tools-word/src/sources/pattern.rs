//! Owns: a date pattern (ooxml-edit's `dates.parse` and `format_date`): the tokens `d dd ddd
//! dddd m mm mmm mmmm mmmmm yy yyyy`, read in either case; text in double quotes or one
//! character after a backslash, and every character that is not a letter, written as given;
//! the names those tokens write the measured ones of the locale's language
//! ([`ooxml_numfmt::measured_names`]), refused where none are measured; and the refusal of
//! a pattern that is none (`date_pattern`), naming the tokens.
//!
//! Must not: read the machine's locale, or guess a name.

use super::Day;
use ooxml_base::pyfmt::repr_str;
use ooxml_numfmt::{measured_names, DateNames, Locale};
use ooxml_tools::{Refusal, ToolError, WayOut};

/// The tokens a pattern takes, as a refusal lists them.
const TOKENS: [&str; 11] = [
    "d", "dd", "ddd", "dddd", "m", "mm", "mmm", "mmmm", "mmmmm", "yy", "yyyy",
];
/// The tokens that write a name.
const NAME_TOKENS: [&str; 5] = ["ddd", "dddd", "mmm", "mmmm", "mmmmm"];
/// What the tokens are, in words.
const TOKENS_TEXT: &str = "d dd (day), ddd dddd (weekday name), m mm (month), mmm mmmm (month name), mmmmm (its \
                           first letter), yy yyyy (year)";

/// A pattern's part.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Token(&'static str),
    Literal(String),
}

/// A date pattern checked for a locale: what a date is written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DatePattern {
    parts: Vec<Part>,
    names: Option<DateNames>,
}

impl DatePattern {
    /// `day` written in the pattern (`format_date`).
    pub(crate) fn format(&self, day: Day) -> String {
        let month = usize::try_from(day.month).unwrap_or(1).saturating_sub(1);
        let weekday = day.weekday();
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Literal(text) => out.push_str(text),
                Part::Token(token) => {
                    let names = self.names;
                    match *token {
                        "d" => out.push_str(&day.day.to_string()),
                        "dd" => out.push_str(&format!("{:02}", day.day)),
                        "ddd" => out.push_str(names.and_then(|n| n.short_days.get(weekday).copied()).unwrap_or("")),
                        "dddd" => out.push_str(names.and_then(|n| n.days.get(weekday).copied()).unwrap_or("")),
                        "m" => out.push_str(&day.month.to_string()),
                        "mm" => out.push_str(&format!("{:02}", day.month)),
                        "mmm" => out.push_str(names.and_then(|n| n.short_months.get(month).copied()).unwrap_or("")),
                        "mmmm" => out.push_str(names.and_then(|n| n.months.get(month).copied()).unwrap_or("")),
                        "mmmmm" => {
                            let full = names.and_then(|n| n.months.get(month).copied()).unwrap_or("");
                            out.extend(full.chars().next());
                        }
                        "yy" => out.push_str(&format!("{:02}", day.year % 100)),
                        _ => out.push_str(&format!("{:04}", day.year)),
                    }
                }
            }
        }
        out
    }
}

/// `pattern` checked as a date pattern for `locale` (`sources.date_pattern`): a pattern
/// that is none, or a name token where the locale's names are not measured, refused as
/// `invalid_arguments` at `field`, the tokens its options.
pub(crate) fn date_pattern(pattern: &str, locale: &Locale, field: &str) -> Result<DatePattern, ToolError> {
    let refuse = |message: String| {
        ToolError::refusal(
            Refusal::InvalidArguments,
            message,
            WayOut::names(TOKENS).unwrap_or_else(|| WayOut::one(serde_json::Value::Null)),
        )
        .at(field)
    };
    let parts = parse(pattern).map_err(refuse)?;
    let names = measured_names(&locale.language);
    if names.is_none() {
        if let Some(Part::Token(used)) = parts
            .iter()
            .find(|p| matches!(p, Part::Token(t) if NAME_TOKENS.contains(t)))
        {
            return Err(refuse(format!(
                "{}: {used} writes a name, and no month or day names are measured for {} (they are for the \
                 languages en, nl, de, fr and it, in any region, and for es-ES, es-MX and es); write the date with \
                 numbers (dd/mm/yyyy, d-m-yyyy, yyyy-mm-dd)",
                repr_str(pattern),
                locale.language
            )));
        }
    }
    Ok(DatePattern { parts, names })
}

/// `pattern` as its parts (`dates.parse`); the message of a pattern that is none.
fn parse(pattern: &str) -> Result<Vec<Part>, String> {
    if pattern.trim().is_empty() {
        return Err(format!(
            "a date pattern is text of the tokens {TOKENS_TEXT}, e.g. d mmmm yyyy"
        ));
    }
    let chars: Vec<char> = pattern.chars().collect();
    let mut parts = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if c == '"' {
            let Some(close) = chars[index + 1..].iter().position(|x| *x == '"').map(|k| k + index + 1) else {
                return Err(format!(
                    "{}: a quote opened at {} is not closed; quote text as \"de\"",
                    repr_str(pattern),
                    index + 1
                ));
            };
            parts.push(Part::Literal(chars[index + 1..close].iter().collect()));
            index = close + 1;
        } else if c == '\\' {
            parts.push(Part::Literal(
                chars.get(index + 1).map(char::to_string).unwrap_or_default(),
            ));
            index += 2;
        } else if matches!(c.to_ascii_lowercase(), 'd' | 'm' | 'y') && c.is_ascii() {
            let lower = c.to_ascii_lowercase();
            let end = (index..chars.len())
                .find(|k| chars[*k].to_ascii_lowercase() != lower || !chars[*k].is_ascii())
                .unwrap_or(chars.len());
            let run: String = chars[index..end].iter().collect::<String>().to_ascii_lowercase();
            let Some(token) = TOKENS.iter().find(|t| **t == run) else {
                return Err(format!(
                    "{}: {} is no date token; the tokens are {TOKENS_TEXT}",
                    repr_str(pattern),
                    repr_str(&chars[index..end].iter().collect::<String>())
                ));
            };
            parts.push(Part::Token(token));
            index = end;
        } else if c.is_alphabetic() {
            return Err(format!(
                "{}: {} is no date token; the tokens are {TOKENS_TEXT}, and other text goes in double quotes \
                 (\"week\")",
                repr_str(pattern),
                repr_str(&c.to_string())
            ));
        } else {
            parts.push(Part::Literal(c.to_string()));
            index += 1;
        }
    }
    if !parts.iter().any(|p| matches!(p, Part::Token(_))) {
        return Err(format!(
            "{} has no date token; the tokens are {TOKENS_TEXT}, e.g. d mmmm yyyy",
            repr_str(pattern)
        ));
    }
    Ok(parts)
}
