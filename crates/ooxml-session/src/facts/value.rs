//! Owns: one value as a reference writes it (ooxml-edit's `textfacts.value_text`,
//! `number_text` and `_general`, ported exactly): text, a number, a logical value or a date
//! as a table cell gets it ([`label`]); a number in a number format, rounded half away from
//! zero on the decimal Python shows for it, the locale's separators ([`number_text`]); a
//! `percent` column's value as a percent; a date in a date pattern with the locale's
//! measured names; and the refusals of a blank, of a value no reference writes, of a
//! format on the wrong kind of value.
//!
//! Must not: write a value's own text into a message (it may be private).

use super::address::{is_date_pattern, number_format, py_strip};
use ooxml_base::pyfmt::{general, repr_float};
use ooxml_numfmt::Locale;
use ooxml_tools::sources::{date_pattern, label, what, Cell, Day, TOKENS_TEXT};
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};

/// A decimal number exactly as Python's `Decimal(repr(value))` holds it: its sign, its
/// coefficient's digits and its exponent.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Decimal {
    negative: bool,
    digits: Vec<u8>,
    exponent: i64,
}

impl Decimal {
    /// The decimal `text` writes (`-12.5`, `1e+20`, `1.5e-07`, `12`).
    fn parse(text: &str) -> Option<Decimal> {
        let (negative, text) = text.strip_prefix('-').map_or((false, text), |t| (true, t));
        let (mantissa, exponent) = match text.split_once(['e', 'E']) {
            Some((m, e)) => (m, e.parse::<i64>().ok()?),
            None => (text, 0),
        };
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let mut digits = Vec::with_capacity(whole.len() + fraction.len());
        for c in whole.chars().chain(fraction.chars()) {
            digits.push(u8::try_from(c.to_digit(10)?).ok()?);
        }
        if digits.is_empty() {
            return None;
        }
        // the coefficient as Python's `Decimal` holds it: no leading zeros
        let zeros = digits.iter().take_while(|d| **d == 0).count().min(digits.len() - 1);
        digits.drain(..zeros);
        let exponent = exponent - i64::try_from(fraction.len()).ok()?;
        Some(Decimal {
            negative,
            digits,
            exponent,
        })
    }

    /// Times 100 (`* 100`: the coefficient two digits longer, as Python's product).
    fn percent(mut self) -> Decimal {
        self.digits.extend([0, 0]);
        self
    }

    /// Rounded to `decimals` places, half away from zero (`quantize`, `ROUND_HALF_UP`): the
    /// digits of its absolute value with exactly `decimals` of them after the point.
    fn quantized(&self, decimals: usize) -> Vec<u8> {
        let decimals_i = i64::try_from(decimals).unwrap_or(0);
        let shift = self.exponent + decimals_i; // the coefficient times 10^shift
        let mut digits = self.digits.clone();
        if shift >= 0 {
            digits.extend(std::iter::repeat_n(0, usize::try_from(shift).unwrap_or(0)));
        } else {
            let cut = usize::try_from(-shift).unwrap_or(0);
            let keep = digits.len().saturating_sub(cut);
            let first_dropped = if cut <= digits.len() {
                digits.get(keep).copied().unwrap_or(0)
            } else {
                0
            };
            digits.truncate(keep);
            if first_dropped >= 5 {
                round_up(&mut digits);
            }
        }
        // at least one digit before the point
        while digits.len() < decimals + 1 {
            digits.insert(0, 0);
        }
        while digits.len() > decimals + 1 && digits.first() == Some(&0) {
            digits.remove(0);
        }
        digits
    }

    /// Whether it is a whole number (`value == value.to_integral()`).
    fn is_integral(&self) -> bool {
        if self.exponent >= 0 {
            return true;
        }
        let cut = usize::try_from(-self.exponent).unwrap_or(usize::MAX);
        self.digits.iter().rev().take(cut).all(|d| *d == 0)
    }

    /// Its exact text in fixed notation (`format(value, "f")`).
    fn fixed(&self) -> String {
        let sign = if self.negative { "-" } else { "" };
        let text: String = self.digits.iter().map(|d| char::from(b'0' + d)).collect();
        if self.exponent >= 0 {
            let zeros = "0".repeat(usize::try_from(self.exponent).unwrap_or(0));
            return format!("{sign}{text}{zeros}");
        }
        let point = usize::try_from(-self.exponent).unwrap_or(0);
        let padded = if text.len() <= point {
            format!("{}{text}", "0".repeat(point + 1 - text.len()))
        } else {
            text
        };
        let (whole, fraction) = padded.split_at(padded.len() - point);
        format!("{sign}{whole}.{fraction}")
    }

    /// Its value as a float (`float(value)`).
    fn to_f64(&self) -> f64 {
        let text: String = self.digits.iter().map(|d| char::from(b'0' + d)).collect();
        let value: f64 = format!("{text}e{}", self.exponent).parse().unwrap_or(0.0);
        if self.negative {
            -value
        } else {
            value
        }
    }
}

/// `digits` plus one in its last place.
fn round_up(digits: &mut Vec<u8>) {
    for digit in digits.iter_mut().rev() {
        if *digit == 9 {
            *digit = 0;
        } else {
            *digit += 1;
            return;
        }
    }
    digits.insert(0, 1);
}

/// The decimal a number cell holds, as Python's `repr` writes it; `None` for any other.
fn decimal_of(value: &Cell) -> Option<Decimal> {
    match value {
        Cell::Int(digits) => Decimal::parse(digits),
        Cell::Float(f) if f.is_finite() => Decimal::parse(&repr_float(*f)),
        _ => None,
    }
}

/// Whether `value` is a finite number (`sources.number`: a logical value is not one).
#[must_use]
pub fn is_number(value: &Cell) -> bool {
    decimal_of(value).is_some()
}

/// `value` in a number format a reference takes (`0.0`, `#,##0`, `0%`), rounded half away
/// from zero on the decimal Python shows for it, separators the locale's (`number_text`);
/// `None` for a format that is none or a value that is no number.
#[must_use]
pub fn number_text(value: &Cell, format: &str, locale: &Locale) -> Option<String> {
    let (decimals, percent) = number_format(format)?;
    let mut exact = decimal_of(value)?;
    if percent {
        exact = exact.percent();
    }
    let digits = exact.quantized(decimals);
    let zero = digits.iter().all(|d| *d == 0);
    let sign = if exact.negative && !zero { "-" } else { "" };
    let text: String = digits.iter().map(|d| char::from(b'0' + d)).collect();
    let (whole, fraction) = text.split_at(text.len() - decimals);
    let whole = if format.starts_with("#,##") {
        grouped(whole, locale.thousands_separator)
    } else {
        whole.to_owned()
    };
    let mut out = format!("{sign}{whole}");
    if decimals > 0 {
        out.push(locale.decimal_separator);
        out.push_str(fraction);
    }
    if percent {
        out.push('%');
    }
    Some(out)
}

/// Whole digits with `group` between each three from the right (`f"{n:,}"`).
fn grouped(whole: &str, group: char) -> String {
    let count = whole.chars().count();
    let mut out = String::with_capacity(whole.len() + count / 3);
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (count - i).is_multiple_of(3) {
            out.push(group);
        }
        out.push(c);
    }
    out
}

/// A percent column's value times 100 as `_general` writes it: a whole number as itself,
/// else to 15 significant digits, the exact decimal where that would take an exponent.
fn general_percent(value: Decimal, locale: &Locale) -> String {
    let text = if value.is_integral() {
        let fixed = value.fixed();
        // `normalize()` then `f`: the whole number, no point
        fixed
            .split_once('.')
            .map_or(fixed.clone(), |(whole, _)| whole.to_owned())
    } else {
        general(value.to_f64(), 15)
    };
    let text = if text.contains(['e', 'E']) { value.fixed() } else { text };
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    text.replace('.', &locale.decimal_separator.to_string())
}

/// The date a value is or names, for a date pattern (`dates.date_of`): a date or a
/// datetime's day, ISO 8601 date text (`2026-03-02`, `2026-03-02T14:05`); `None` for a time
/// of day alone or anything else.
#[must_use]
pub fn date_of(value: &Cell) -> Option<Day> {
    match value {
        Cell::Date(day) | Cell::DateTime(day, _) => Some(*day),
        Cell::Text(text) => iso_date(py_strip(text)),
        _ => None,
    }
}

/// `YYYY-MM-DD`, then optionally `T` or a space and `HH:MM`, `:SS` optional
/// (`dates._ISO_DATE`), whose day and time exist.
fn iso_date(text: &str) -> Option<Day> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let digits = |part: Option<&str>, n: usize| -> Option<u32> {
        let part = part?;
        (part.len() == n && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.parse().ok())?
    };
    let day = Day::new(
        i32::try_from(digits(text.get(0..4), 4)?).ok()?,
        digits(text.get(5..7), 2)?,
        digits(text.get(8..10), 2)?,
    );
    if text.get(4..5) != Some("-") || text.get(7..8) != Some("-") {
        return None;
    }
    let rest = text.get(10..)?;
    if rest.is_empty() {
        return day;
    }
    let time = rest.strip_prefix(['T', ' '])?;
    let (hour, minute, second) = match time.len() {
        5 => (digits(time.get(0..2), 2)?, digits(time.get(3..5), 2)?, 0),
        8 => (
            digits(time.get(0..2), 2)?,
            digits(time.get(3..5), 2)?,
            digits(time.get(6..8), 2)?,
        ),
        _ => return None,
    };
    if time.get(2..3) != Some(":") || (time.len() == 8 && time.get(5..6) != Some(":")) {
        return None;
    }
    (hour < 24 && minute < 60 && second < 60).then_some(day).flatten()
}

/// A value as a reference writes it (`value_text`): a blank or anything that is not text,
/// a number, a logical value or a date is refused, naming `place`; a format on the wrong
/// kind of value too.
///
/// `how`: the value's column type label and the number format or date pattern given.
///
/// # Errors
/// `invalid_arguments` at `field`, as above, `retry` its way out.
pub fn value_text(
    value: &Cell,
    locale: &Locale,
    how: (Option<&str>, Option<&str>),
    place: &str,
    field: &str,
    retry: &TryCall,
) -> Result<String, ToolError> {
    let (kind, format) = how;
    let refuse =
        |message: String| ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(retry.clone())).at(field);
    if value.is_blank() {
        return Err(refuse(format!("{place} is blank: a reference writes a value")));
    }
    if let Some(format) = format.filter(|f| is_date_pattern(Some(f))) {
        let Some(day) = date_of(value) else {
            let other = if is_number(value) {
                " -- for a number write :0, :0.0 or :#,##0 --"
            } else {
                ""
            };
            return Err(refuse(format!(
                "{place} is {}: a date pattern (:{format}) is for dates{other}; leave it out",
                what(value)
            )));
        };
        return Ok(date_pattern(format, locale, field)?.format(day));
    }
    if let Some(format) = format {
        return number_text(value, format, locale).ok_or_else(|| {
            let instead = if date_of(value).is_some() {
                format!("; for a date write a date pattern instead, of {TOKENS_TEXT}: :d mmmm yyyy, :dd/mm/yyyy")
            } else {
                "; leave it out".to_owned()
            };
            refuse(format!(
                "{place} is {}: a number format (:{format}) is for numbers{instead}",
                what(value)
            ))
        });
    }
    if kind == Some("percent") {
        if let Some(exact) = decimal_of(value) {
            return Ok(general_percent(exact.percent(), locale) + "%");
        }
    }
    label(value, locale, kind, None).ok_or_else(|| {
        refuse(format!(
            "{place} is {}: a reference writes text, a number, a logical value or a date",
            what(value)
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_numfmt::EN_US;

    fn nl_be() -> Locale {
        Locale::new(',', '.', "dmy", "nl-BE", ';').unwrap()
    }

    /// `test_rounding_is_half_away_from_zero_on_the_decimal_shown`.
    #[test]
    fn rounding_is_half_away_from_zero_on_the_decimal_shown() {
        let text = |v: f64, f: &str| number_text(&Cell::Float(v), f, &EN_US).unwrap();
        assert_eq!(text(2.675, "0.00"), "2.68");
        assert_eq!(text(-2.5, "0"), "-3");
        assert_eq!(text(-0.04, "0.0"), "0.0");
        assert_eq!(text(0.125, "0%"), "13%");
        assert_eq!(number_text(&Cell::Float(1234.5), "#,##0", &nl_be()).unwrap(), "1.235");
        assert_eq!(text(1_234_567.891, "#,##0.0"), "1,234,567.9");
        assert_eq!(text(0.253, "0.0%"), "25.3%");
        assert_eq!(text(1e20, "0"), "100000000000000000000");
        assert_eq!(
            number_text(&Cell::Int("12345678901234567890".into()), "#,##0", &EN_US).unwrap(),
            "12,345,678,901,234,567,890"
        );
        assert_eq!(text(0.000_15, "0.000"), "0.000");
        assert_eq!(text(9.995, "0.00"), "10.00");
        assert_eq!(text(99.95, "0.0"), "100.0");
    }

    #[test]
    fn a_percent_column_writes_its_value_as_a_percent() {
        let retry = TryCall {
            tool: "t".into(),
            arguments: serde_json::Map::new(),
            fill: vec!["f".into()],
        };
        let percent =
            |v: f64, l: &Locale| value_text(&Cell::Float(v), l, (Some("percent"), None), "x", "f", &retry).unwrap();
        assert_eq!(percent(0.253, &nl_be()), "25,3%");
        assert_eq!(percent(0.5, &EN_US), "50%");
        assert_eq!(percent(1.0, &EN_US), "100%");
        assert_eq!(percent(1e-7, &EN_US), "0.00001%");
    }

    #[test]
    fn a_date_is_read_from_iso_text() {
        let day = Day::new(2026, 3, 2);
        assert_eq!(date_of(&Cell::Text("2026-03-02".into())), day);
        assert_eq!(date_of(&Cell::Text(" 2026-03-02T14:05 ".into())), day);
        assert_eq!(date_of(&Cell::Text("2026-03-02 14:05:09".into())), day);
        assert_eq!(date_of(&Cell::Text("2026-03-02T25:05".into())), None);
        assert_eq!(date_of(&Cell::Text("2026-02-30".into())), None);
        assert_eq!(date_of(&Cell::Text("14:05".into())), None);
        assert_eq!(date_of(&Cell::Float(2.0)), None);
    }
}
