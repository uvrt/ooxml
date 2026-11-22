//! Owns: a source value as a cell's text (ooxml-edit's `sources.label`, as a chart's
//! category label writes it): text as it is; a number as the General format writes it with
//! the locale's decimal separator; a logical value `TRUE` or `FALSE`; a date, a datetime or
//! a time -- and the ISO 8601 text of a `date`, `datetime` or `time` column -- in the
//! locale's day order (`moment_text`); a date in a pattern where one is given; and a value's
//! kind for a message (`what`).
//!
//! Must not: write a value's own text into a message (it may be private).

use super::{Clock, DatePattern, Day, SourceValue};
use ooxml_numfmt::Locale;

/// The column type labels whose ISO 8601 text is written as a date or a time.
const MOMENT_TYPES: [&str; 3] = ["date", "datetime", "time"];

/// A date, a datetime or a time of day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Moment {
    Date(Day),
    DateTime(Day, Clock),
    Time(Clock),
}

/// The value as a category label or a cell's text (`label`); `None` for a blank, or a
/// value no cell takes.
pub(crate) fn label(
    value: &SourceValue,
    locale: &Locale,
    kind: Option<&str>,
    pattern: Option<&DatePattern>,
) -> Option<String> {
    if let Some(pattern) = pattern {
        if let Some(day) = date_value(value, kind) {
            return Some(pattern.format(day));
        }
    }
    match value {
        SourceValue::Text(text) => {
            if kind.is_some_and(|k| MOMENT_TYPES.contains(&k)) {
                if let Some(found) = moment_of(text) {
                    return Some(moment_text(found, locale));
                }
            }
            (!text.is_empty()).then(|| text.clone())
        }
        SourceValue::Bool(b) => Some(if *b { "TRUE" } else { "FALSE" }.to_owned()),
        SourceValue::Date(day) => Some(moment_text(Moment::Date(*day), locale)),
        SourceValue::DateTime(day, clock) => Some(moment_text(Moment::DateTime(*day, *clock), locale)),
        SourceValue::Time(clock) => Some(moment_text(Moment::Time(*clock), locale)),
        SourceValue::Int(digits) => Some(digits.clone()),
        SourceValue::Float(value) => Some(number_text(*value, locale)),
        SourceValue::Blank | SourceValue::Other(_) => None,
    }
}

/// A finite number as `label` writes it: a whole one as an integer, else to 15 significant
/// digits, the locale's decimal separator for `.`.
fn number_text(value: f64, locale: &Locale) -> String {
    #[allow(clippy::cast_possible_truncation)]
    let text = if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        ooxml_chart::pyfmt::general(value, 15)
    };
    if locale.decimal_separator == '.' {
        text
    } else {
        text.replace('.', &locale.decimal_separator.to_string())
    }
}

/// The value when it is a date, for a date pattern (`date_value`): a date or a datetime's
/// day, or the ISO 8601 text of a `date` or `datetime` column's value.
pub(crate) fn date_value(value: &SourceValue, kind: Option<&str>) -> Option<Day> {
    match value {
        SourceValue::Date(day) | SourceValue::DateTime(day, _) => Some(*day),
        SourceValue::Text(text) if matches!(kind, Some("date" | "datetime")) => match moment_of(text)? {
            Moment::Date(day) | Moment::DateTime(day, _) => Some(day),
            Moment::Time(_) => None,
        },
        _ => None,
    }
}

/// A value's kind for a message (`what`), never the value itself.
pub(crate) fn what(value: &SourceValue) -> String {
    match value {
        SourceValue::Blank => "blank".to_owned(),
        SourceValue::Bool(_) => "a logical value".to_owned(),
        SourceValue::Text(t) if t.trim().is_empty() => "empty text".to_owned(),
        SourceValue::Text(_) => "text".to_owned(),
        SourceValue::Int(_) => "a int".to_owned(),
        SourceValue::Float(_) => "a float".to_owned(),
        SourceValue::Date(_) => "a date".to_owned(),
        SourceValue::DateTime(..) => "a datetime".to_owned(),
        SourceValue::Time(_) => "a time".to_owned(),
        SourceValue::Other(kind) => kind.clone(),
    }
}

/// A moment as text in the locale's day order (`moment_text`): a date's day, month and
/// four-digit year as digits joined by `/` (`mdy`, `dmy`), else ISO 8601; a time of day on
/// the 24-hour clock, its seconds where it has any; a datetime the two, a space between.
fn moment_text(moment: Moment, locale: &Locale) -> String {
    match moment {
        Moment::DateTime(day, clock) => format!(
            "{} {}",
            moment_text(Moment::Date(day), locale),
            moment_text(Moment::Time(clock), locale)
        ),
        Moment::Time(clock) => {
            let shown = format!("{:02}:{:02}", clock.hour, clock.minute);
            if clock.second == 0 {
                shown
            } else {
                format!("{shown}:{:02}", clock.second)
            }
        }
        Moment::Date(day) => match locale.date_order.as_ref() {
            "dmy" => format!("{}/{}/{}", day.day, day.month, day.year),
            "mdy" => format!("{}/{}/{}", day.month, day.day, day.year),
            _ => format!("{:04}-{:02}-{:02}", day.year, day.month, day.day),
        },
    }
}

/// The moment ISO 8601 text names (`moment_of`, not as written in a locale): `2026-03-01`,
/// `2026-03-01T14:05:00` or with a space, `14:05`, seconds and their fraction optional;
/// `None` for anything else, or a day or time that does not exist.
fn moment_of(text: &str) -> Option<Moment> {
    let text = python_strip(text);
    let b = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| -> Option<u32> {
        let part = text.get(range)?;
        (!part.is_empty() && part.bytes().all(|c| c.is_ascii_digit()))
            .then(|| part.parse().ok())
            .flatten()
    };
    if b.len() >= 10 && b.get(4) == Some(&b'-') && b.get(7) == Some(&b'-') {
        let day = Day::new(i32::try_from(digits(0..4)?).ok()?, digits(5..7)?, digits(8..10)?)?;
        if b.len() == 10 {
            return Some(Moment::Date(day));
        }
        if !matches!(b.get(10), Some(b'T' | b' ')) {
            return None;
        }
        let rest = text.get(11..)?;
        // the clock after a date takes two-digit hours
        if rest.as_bytes().get(2) != Some(&b':') {
            return None;
        }
        return clock(rest).map(|c| Moment::DateTime(day, c));
    }
    clock(text).map(Moment::Time)
}

/// `H:MM`, `HH:MM`, seconds `:SS` and a fraction `.f` (1 to 6 digits) optional.
fn clock(text: &str) -> Option<Clock> {
    let (main, fraction) = match text.split_once('.') {
        Some((main, fraction)) => (main, Some(fraction)),
        None => (text, None),
    };
    let parts: Vec<&str> = main.split(':').collect();
    let two = |part: &str| part.len() == 2 && part.bytes().all(|c| c.is_ascii_digit());
    let (hour, minute, second) = match parts.as_slice() {
        [h, m] if fraction.is_none() => (*h, *m, None),
        [h, m, s] => (*h, *m, Some(*s)),
        _ => return None,
    };
    if !(1..=2).contains(&hour.len()) || !hour.bytes().all(|c| c.is_ascii_digit()) || !two(minute) {
        return None;
    }
    if second.is_some_and(|s| !two(s)) {
        return None;
    }
    let micro = match fraction {
        Some(f) if (1..=6).contains(&f.len()) && f.bytes().all(|c| c.is_ascii_digit()) => {
            format!("{f:0<6}").parse().ok()?
        }
        Some(_) => return None,
        None => 0,
    };
    Clock::new(
        hour.parse().ok()?,
        minute.parse().ok()?,
        second.map_or(Some(0), |s| s.parse().ok())?,
        micro,
    )
}

/// Python's `str.strip()`: white space as Python reads it off both ends.
fn python_strip(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}' | '\u{85}'))
}
