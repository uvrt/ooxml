//! Owns: numbers written and rounded as the Python reference writes and rounds them --
//! `repr(float)` (the shortest string that reads back, exponent outside 1e-4 to 1e16),
//! `round(value, digits)` (the exact binary value, ties to even), `f"{value:.Nf}"` and
//! `f"{value:.Ng}"`. Text the layout measures and paths it writes depend on these digits,
//! so they are the reference's, not Rust's own.
//!
//! Must not: write any other format.

/// `round(value, digits)`: the decimal nearest the exact value, ties to even, read back.
pub fn round(value: f64, digits: usize) -> f64 {
    if !value.is_finite() {
        return value;
    }
    format!("{value:.digits$}").parse().unwrap_or(value)
}

/// Python's `round(value)` to an integer: ties to even.
pub fn round_half_even(value: f64) -> f64 {
    let rounded = value.round();
    if (value - value.trunc()).abs() == 0.5 {
        2.0 * (value / 2.0).round()
    } else {
        rounded
    }
}

/// `f"{value:.{digits}f}"`.
pub fn fixed(value: f64, digits: usize) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf".into() } else { "-inf".into() };
    }
    format!("{value:.digits$}")
}

/// The shortest round-trip digits of a finite non-zero `value` and its decimal exponent:
/// `(digits, exponent)` with the value `0.d1d2... * 10^(exponent + 1)`.
fn shortest(value: f64) -> (String, i32) {
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    (digits, exponent.parse().unwrap_or(0))
}

/// `repr(value)` of a float.
pub fn repr(value: f64) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value == 0.0 {
        return format!("{sign}0.0");
    }
    let (digits, exponent) = shortest(value);
    if !(-4..16).contains(&exponent) {
        let (first, rest) = digits.split_at(1);
        let mantissa = if rest.is_empty() {
            first.to_owned()
        } else {
            format!("{first}.{rest}")
        };
        let mark = if exponent < 0 { '-' } else { '+' };
        return format!("{sign}{mantissa}e{mark}{:02}", exponent.abs());
    }
    format!("{sign}{}", place_point(&digits, exponent, true))
}

/// `digits` (a value `d.ddd * 10^exponent`) written in positional notation; with
/// `point_zero`, a whole number gets `.0`.
fn place_point(digits: &str, exponent: i32, point_zero: bool) -> String {
    let length = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    if exponent < 0 {
        let zeros = usize::try_from(-exponent - 1).unwrap_or(0);
        return format!("0.{}{digits}", "0".repeat(zeros));
    }
    if exponent + 1 >= length {
        let zeros = usize::try_from(exponent + 1 - length).unwrap_or(0);
        let tail = if point_zero { ".0" } else { "" };
        return format!("{digits}{}{tail}", "0".repeat(zeros));
    }
    let split = usize::try_from(exponent + 1).unwrap_or(0);
    let (whole, fraction) = digits.split_at(split.min(digits.len()));
    format!("{whole}.{fraction}")
}

/// `str(value)` of a float: its `repr`.
pub fn str(value: f64) -> String {
    repr(value)
}

/// `f"{value:.{precision}g}"` (`precision` 0 is taken as 1); `"{value:g}"` is precision 6.
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
    use super::{fixed, general, repr, round, round_half_even};

    #[test]
    fn repr_writes_as_python_does() {
        for (value, written) in [
            (0.25, "0.25"),
            (1e-7, "1e-07"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (123_456.0, "123456.0"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (-2.5, "-2.5"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1.2345678901234568e17, "1.2345678901234568e+17"),
            (-0.0, "-0.0"),
        ] {
            assert_eq!(repr(value), written, "{value}");
        }
    }

    #[test]
    fn general_writes_as_python_does() {
        for (value, precision, written) in [
            (0.0, 6, "0"),
            (1234567.0, 6, "1.23457e+06"),
            (0.0001, 6, "0.0001"),
            (0.00001234, 6, "1.234e-05"),
            (12.5, 6, "12.5"),
            (100.0, 6, "100"),
            (2.0 / 3.0, 3, "0.667"),
        ] {
            assert_eq!(general(value, precision), written, "{value} {precision}");
        }
    }

    #[test]
    fn rounding_is_python_rounding() {
        assert_eq!(fixed(0.125, 2), "0.12");
        assert_eq!(round(2.675, 2), 2.67);
        assert_eq!(round_half_even(2.5), 2.0);
        assert_eq!(round_half_even(3.5), 4.0);
        assert_eq!(round_half_even(-2.5), -2.0);
    }
}
