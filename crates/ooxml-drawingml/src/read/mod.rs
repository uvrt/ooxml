//! Owns: reading DrawingML from a part's tree into this crate's source model, equal to
//! ooxml-common's readers (`drawingml/read.py`, `read_text.py`, `read_tree.py`): colours
//! ([`colour`]), fills, lines and effects ([`fill`]), transforms and geometry
//! ([`geometry`]), the theme and colour maps ([`theme`]), text bodies ([`text`]) and shape
//! trees ([`tree`]). Elements are found by local name through `ooxml_schema::a`'s views;
//! values are taken as the Python readers take them (this module's helpers say how).
//!
//! Must not: resolve anything (`crate::resolve`), or know which application wrote the part.

pub mod colour;
pub mod fill;
pub mod geometry;
pub mod text;
pub mod theme;
pub mod tree;

use ooxml_xml::Element;

/// The relationships namespace, in which `r:embed` and `r:id` are read exactly.
pub const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// Python's `value or default` for a number read from an attribute: an absent value or 0
/// is the default.
pub(crate) fn or(value: Option<f64>, default: f64) -> f64 {
    match value {
        Some(v) if v != 0.0 => v,
        _ => default,
    }
}

/// Python's `value or default` for a string: an absent or empty value is the default.
pub(crate) fn or_str(value: Option<&str>, default: &str) -> String {
    value.filter(|v| !v.is_empty()).unwrap_or(default).to_owned()
}

/// Python's `value or None` for a string: an empty value is none.
pub(crate) fn non_empty(value: Option<&str>) -> Option<String> {
    value.filter(|v| !v.is_empty()).map(str::to_owned)
}

/// ooxml-common's `is_true`: `1` or `true`, exactly.
pub(crate) fn is_true(value: Option<&str>) -> bool {
    matches!(value, Some("1" | "true"))
}

/// `None` where absent, else [`is_true`].
pub(crate) fn optional_bool(value: Option<&str>) -> Option<bool> {
    value.map(|v| is_true(Some(v)))
}

/// Python's `int()` of a float: toward zero (saturating where it would not fit).
pub(crate) fn int_of(value: f64) -> i64 {
    // Truncation toward zero is the point; `as` saturates at the bounds.
    #[allow(clippy::cast_possible_truncation)]
    let int = value.trunc() as i64;
    int
}

/// An attribute's string, owned.
pub(crate) fn owned(value: Option<&str>) -> Option<String> {
    value.map(str::to_owned)
}

/// A numeric attribute of an optional element (`None` where the element is absent).
pub(crate) fn num(element: Option<&Element>, name: &str) -> Option<f64> {
    element.and_then(|e| ooxml_xml::view::attr_num(e, name))
}

/// Numeric character references left in an attribute value (`&#x2022;` as text) decoded,
/// as ooxml-common's `decode_char_refs` does: `&#x` hex digits `;` and `&#` decimal digits
/// `;`. A reference to no character is left as it is.
pub(crate) fn decode_char_refs(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("&#") {
        out.push_str(rest.get(..start).unwrap_or(""));
        let tail = rest.get(start + 2..).unwrap_or("");
        let (hex, digits_from) = match tail.as_bytes().first() {
            Some(b'x') => (true, 1),
            _ => (false, 0),
        };
        let digits: &str = tail.get(digits_from..).unwrap_or("");
        let len = digits
            .bytes()
            .take_while(|b| if hex { b.is_ascii_hexdigit() } else { b.is_ascii_digit() })
            .count();
        let after = digits.get(len..).unwrap_or("");
        let decoded = (len > 0 && after.starts_with(';'))
            .then(|| {
                digits
                    .get(..len)
                    .and_then(|d| u32::from_str_radix(d, if hex { 16 } else { 10 }).ok())
            })
            .flatten()
            .and_then(char::from_u32);
        match decoded {
            Some(c) => {
                out.push(c);
                rest = after.get(1..).unwrap_or("");
            }
            None => {
                out.push_str("&#");
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_references_decode_as_python_decodes_them() {
        assert_eq!(decode_char_refs("a&#x2022;b&#8226;c"), "a\u{2022}b\u{2022}c");
        assert_eq!(decode_char_refs("&#x;&#12"), "&#x;&#12");
        assert_eq!(or(Some(0.0), 5.0), 5.0);
        assert_eq!(int_of(-2.7), -2);
    }
}
