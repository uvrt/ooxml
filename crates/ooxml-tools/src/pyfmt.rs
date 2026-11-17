//! Values written as today's Python tool layer writes them in its messages: `repr` of a
//! string (`'one'`), of a JSON value (`True`, `None`, `['a', 2]`) and of a float (`0.06`,
//! `1e-05`, `2.0`).
//!
//! Owns: those formats (a string's `repr` is `ooxml-base`'s, re-exported), so that a message reads the same from the Rust core as from the
//! Python path it replaces (the differential compares them).
//!
//! Must not: be used for anything but messages. `str.isprintable` is approximated without
//! Unicode tables: control characters, line and paragraph separators, the Unicode spaces
//! other than U+0020 and the common format characters are escaped.

pub use ooxml_base::pyfmt::repr_str;
use serde_json::Value;

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

/// A JSON number as Python prints it: a whole number as one, a float by [`repr_float`].
#[must_use]
pub fn number(value: &serde_json::Number) -> String {
    if value.is_f64() {
        repr_float(value.as_f64().unwrap_or(0.0))
    } else {
        value.to_string()
    }
}

/// Python's `repr` of a JSON value as the json module reads it.
#[must_use]
pub fn repr(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => number(n),
        Value::String(s) => repr_str(s),
        Value::Array(items) => format!("[{}]", items.iter().map(repr).collect::<Vec<_>>().join(", ")),
        Value::Object(map) => {
            let pairs: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}: {}", repr_str(k), repr(v)))
                .collect();
            format!("{{{}}}", pairs.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_read_as_python_writes_them() {
        for (value, text) in [
            (0.06, "0.06"),
            (2.0, "2.0"),
            (1e-5, "1e-05"),
            (1e16, "1e+16"),
            (123_456.5, "123456.5"),
            (-0.5, "-0.5"),
            (1.5e300, "1.5e+300"),
            (0.0001, "0.0001"),
        ] {
            assert_eq!(repr_float(value), text);
        }
    }

    #[test]
    fn strings_read_as_python_writes_them() {
        assert_eq!(repr_str("one"), "'one'");
        assert_eq!(repr_str("it's"), "\"it's\"");
        assert_eq!(repr_str("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(repr_str("a\nb\u{b}"), "'a\\nb\\x0b'");
        assert_eq!(repr(&serde_json::json!([true, null, "x", 2])), "[True, None, 'x', 2]");
    }
}
