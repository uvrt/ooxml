//! Owns: a string as Python's `repr` writes it (`'one'`, `"it's"`), for messages that must
//! read the same from the Rust core as from the Python code it replaces (the differentials
//! compare them); and a float as Python's `repr` writes it (`0.06`, `1e-05`, `2.0`), which
//! is also how Python's `json.dumps` writes one; and Python's `round(value, 2)`.
//!
//! Must not: be used for anything but messages and JSON written as Python writes it.
//! `str.isprintable` is approximated without Unicode tables: control characters, line and
//! paragraph separators, the Unicode spaces other than U+0020 and the common format
//! characters are escaped.

use std::fmt::Write as _;

/// Python's `repr` of a string.
#[must_use]
pub fn repr_str(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if printable(c) => out.push(c),
            c => escape(c, &mut out),
        }
    }
    out.push(quote);
    out
}

fn escape(c: char, out: &mut String) {
    let code = u32::from(c);
    // Writing to a String cannot fail.
    let _ = if code <= 0xff {
        write!(out, "\\x{code:02x}")
    } else if code <= 0xffff {
        write!(out, "\\u{code:04x}")
    } else {
        write!(out, "\\U{code:08x}")
    };
}

fn printable(c: char) -> bool {
    if c == ' ' {
        return true;
    }
    if c.is_control() {
        return false;
    }
    !matches!(
        u32::from(c),
        0xa0 | 0xad | 0x1680 | 0x2000..=0x200f | 0x2028..=0x202f | 0x205f..=0x2064 | 0x3000 | 0xfeff
    )
}

/// Python's `repr` of a float.
#[must_use]
pub fn repr_float(value: f64) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.into();
    }
    // `{:e}` is the shortest form that reads back as the same float, as Python's repr.
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let sign = if negative { "-" } else { "" };
    if (-4..16).contains(&exponent) {
        let point = exponent + 1;
        let fixed = fixed_digits(&digits, point);
        return format!("{sign}{fixed}");
    }
    let (head, tail) = digits.split_at(1.min(digits.len()));
    let fraction = if tail.is_empty() {
        String::new()
    } else {
        format!(".{tail}")
    };
    let exp_sign = if exponent < 0 { '-' } else { '+' };
    format!("{sign}{head}{fraction}e{exp_sign}{:02}", exponent.abs())
}

/// Python's `{:g}` for the values a width or a length in points takes: a whole number as
/// itself, else six decimals with the trailing zeros dropped.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn g(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e16 {
        format!("{}", value as i64)
    } else {
        let text = format!("{value:.6}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// Python's `round(value, 2)`: the nearest of two decimals, ties to even on the exact value.
#[must_use]
pub fn round2(value: f64) -> f64 {
    format!("{value:.2}").parse().unwrap_or(value)
}

/// `digits` with the decimal point after `point` of them (it may fall before the first or
/// after the last), always with a fraction (`2.0`).
fn fixed_digits(digits: &str, point: i32) -> String {
    let count = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    if point <= 0 {
        let zeros = "0".repeat(usize::try_from(-point).unwrap_or(0));
        return format!("0.{zeros}{digits}");
    }
    if point >= count {
        let zeros = "0".repeat(usize::try_from(point - count).unwrap_or(0));
        return format!("{digits}{zeros}.0");
    }
    let at = usize::try_from(point).unwrap_or(0);
    let (whole, fraction) = digits.split_at(at.min(digits.len()));
    format!("{whole}.{fraction}")
}

/// A number as Python's `json` module reads it: a JSON integer is an `int`, anything with a
/// fraction or an exponent a `float`. Python writes the two apart (`5` and `5.0`), so a
/// message that names a number from the arguments keeps which it was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PyNumber {
    /// An `int` (JSON's `5`).
    Int(i128),
    /// A `float` (JSON's `5.0`, `5e0`).
    Float(f64),
}

impl PyNumber {
    /// The number's value.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // as Python's float(int)
    pub fn value(self) -> f64 {
        match self {
            Self::Int(int) => int as f64,
            Self::Float(float) => float,
        }
    }
}

impl From<f64> for PyNumber {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<i64> for PyNumber {
    fn from(value: i64) -> Self {
        Self::Int(value.into())
    }
}

/// Python's `str` of the number: `5`, `5.0`, `1e-05`.
impl std::fmt::Display for PyNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int(int) => write!(f, "{int}"),
            Self::Float(float) => f.write_str(&repr_float(*float)),
        }
    }
}

/// `f"{value:.{precision}g}"` (`precision` 0 is taken as 1); `"{value:g}"` is precision 6.
#[must_use]
pub fn general(value: f64, precision: usize) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let precision = precision.max(1);
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value == 0.0 {
        return format!("{sign}0");
    }
    let scientific = format!("{:.*e}", precision - 1, value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let limit = i32::try_from(precision).unwrap_or(i32::MAX);
    if exponent < -4 || exponent >= limit {
        let mantissa = trim_fraction(mantissa);
        let mark = if exponent < 0 { '-' } else { '+' };
        return format!("{sign}{mantissa}e{mark}{:02}", exponent.abs());
    }
    let decimals = usize::try_from(limit - 1 - exponent).unwrap_or(0);
    format!("{sign}{}", trim_fraction(&format!("{:.decimals$}", value.abs())))
}

/// A number's trailing fractional zeros (and a bare point) removed.
fn trim_fraction(text: &str) -> &str {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::{repr_float, repr_str, round2, PyNumber};

    #[test]
    fn numbers_keep_int_and_float_apart() {
        assert_eq!(PyNumber::from(5).to_string(), "5");
        assert_eq!(PyNumber::from(5.0).to_string(), "5.0");
        assert_eq!(PyNumber::Float(1e-5).to_string(), "1e-05");
        assert!((PyNumber::from(-3).value() + 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn strings_read_as_python_writes_them() {
        assert_eq!(repr_str("one"), "'one'");
        assert_eq!(repr_str("it's"), "\"it's\"");
        assert_eq!(repr_str("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(repr_str("a\nb\u{b}"), "'a\\nb\\x0b'");
    }

    #[test]
    fn floats_read_as_python_writes_them() {
        assert_eq!(repr_float(2.0), "2.0");
        assert_eq!(repr_float(0.06), "0.06");
        assert_eq!(repr_float(1e-5), "1e-05");
        assert_eq!(repr_float(1e16), "1e+16");
        assert_eq!(repr_float(-1.5), "-1.5");
    }

    #[test]
    fn rounds_to_two_decimals_as_python() {
        assert!((round2(1.005) - 1.0).abs() < 1e-12);
        assert!((round2(2.675) - 2.67).abs() < 1e-12);
        assert!((round2(12.3456) - 12.35).abs() < 1e-12);
    }
}
