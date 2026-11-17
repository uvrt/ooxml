//! Owns: a string as Python's `repr` writes it (`'one'`, `"it's"`), for messages that must
//! read the same from the Rust core as from the Python code it replaces (the differentials
//! compare them).
//!
//! Must not: be used for anything but messages. `str.isprintable` is approximated without
//! Unicode tables: control characters, line and paragraph separators, the Unicode spaces
//! other than U+0020 and the common format characters are escaped.

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

#[cfg(test)]
mod tests {
    use super::repr_str;

    #[test]
    fn strings_read_as_python_writes_them() {
        assert_eq!(repr_str("one"), "'one'");
        assert_eq!(repr_str("it's"), "\"it's\"");
        assert_eq!(repr_str("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(repr_str("a\nb\u{b}"), "'a\\nb\\x0b'");
    }
}
