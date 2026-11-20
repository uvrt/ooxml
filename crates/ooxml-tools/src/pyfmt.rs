//! Values written as today's Python tool layer writes them in its messages: `repr` of a
//! string (`'one'`), of a JSON value (`True`, `None`, `['a', 2]`) and of a float (`0.06`,
//! `1e-05`, `2.0`).
//!
//! Owns: those formats (a string's and a float's `repr` are `ooxml-base`'s, re-exported), so that a message reads the same from the Rust core as from the
//! Python path it replaces (the differential compares them).
//!
//! Must not: be used for anything but messages. `str.isprintable` is approximated without
//! Unicode tables: control characters, line and paragraph separators, the Unicode spaces
//! other than U+0020 and the common format characters are escaped.

pub use ooxml_base::pyfmt::{repr_float, repr_str};
use serde_json::Value;

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

/// Whether Python's `float(text)` reads `text` as a number: surrounding white space,
/// a sign, `inf`, `infinity` and `nan` in any case, and single underscores between
/// digits (PEP 515) allowed.
#[must_use]
pub fn is_float(text: &str) -> bool {
    let text = text.trim();
    let chars: Vec<char> = text.chars().collect();
    let between_digits = |k: usize| {
        k > 0
            && chars.get(k - 1).is_some_and(char::is_ascii_digit)
            && chars.get(k + 1).is_some_and(char::is_ascii_digit)
    };
    if chars.iter().enumerate().any(|(k, c)| *c == '_' && !between_digits(k)) {
        return false;
    }
    let plain: String = chars.into_iter().filter(|c| *c != '_').collect();
    !plain.is_empty() && plain.parse::<f64>().is_ok()
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

    #[test]
    fn numbers_as_python_reads_them() {
        for yes in ["5", " 2.5 ", "1_000", "-inf", "NaN", "1e3", ".5", "+7."] {
            assert!(super::is_float(yes), "{yes}");
        }
        for no in ["region", "", "1__0", "_1", "1_", "1e", "total 5"] {
            assert!(!super::is_float(no), "{no}");
        }
    }
}
