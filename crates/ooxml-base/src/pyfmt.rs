//! Owns: a string as Python's `repr` writes it (`'one'`, `"it's"`), for messages that must
//! read the same from the Rust core as from the Python code it replaces (the differentials
//! compare them); and a float as Python's `repr` writes it (`0.06`, `1e-05`, `2.0`), which
//! is also how Python's `json.dumps` writes one.
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

#[cfg(test)]
mod tests {
    use super::{repr_float, repr_str};

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
}
