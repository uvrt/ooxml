//! Owns: how the model reads an element -- the namespaces its names are matched in
//! ([`Ns`]: one per vocabulary, a transitional URI and, where ISO/IEC 29500 Strict names
//! the vocabulary otherwise, the strict one, so `Ns::W` holds transitional and strict
//! WordprocessingML alike); children and attributes in exactly their namespace (the
//! generated views, `crate::w`, read so), or by **local name** whatever the namespace (the
//! readers not yet ported onto the views); integers as docx2svg reads them, and
//! WordprocessingML's on/off values.
//!
//! Must not: know an element's meaning, or allocate where a borrow does.

use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// A vocabulary's namespace as the Word readers match names in it: its transitional URI,
/// and the URI ISO/IEC 29500 Strict gives it where that differs (a strict document's parts
/// are written in those). A name in either is in the vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ns {
    uri: &'static str,
    strict: Option<&'static str>,
}

impl Ns {
    /// WordprocessingML (`w:`).
    pub const W: Ns = Ns::strict_too(crate::parts::W, "http://purl.oclc.org/ooxml/wordprocessingml/main");
    /// Relationships named from a part (`r:id`, `r:embed`).
    pub const R: Ns = Ns::strict_too(
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
        "http://purl.oclc.org/ooxml/officeDocument/relationships",
    );
    /// DrawingML's placing of a drawing in a document (`wp:`).
    pub const WP: Ns = Ns::strict_too(
        crate::ids::WP,
        "http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing",
    );
    /// DrawingML (`a:`).
    pub const A: Ns = Ns::strict_too(
        "http://schemas.openxmlformats.org/drawingml/2006/main",
        "http://purl.oclc.org/ooxml/drawingml/main",
    );
    /// Word 2010's (`w14:`): the same in a strict document.
    pub const W14: Ns = Ns::only(crate::parts::W14);
    /// Word 2010's shapes (`wps:`): the same in a strict document.
    pub const WPS: Ns = Ns::only("http://schemas.microsoft.com/office/word/2010/wordprocessingShape");
    /// Word 2010's groups (`wpg:`): the same in a strict document.
    pub const WPG: Ns = Ns::only("http://schemas.microsoft.com/office/word/2010/wordprocessingGroup");

    /// A namespace one URI names, in transitional and strict documents alike.
    #[must_use]
    pub const fn only(uri: &'static str) -> Ns {
        Ns { uri, strict: None }
    }

    /// A namespace `uri` names, and `strict` in a strict document.
    #[must_use]
    pub const fn strict_too(uri: &'static str, strict: &'static str) -> Ns {
        Ns {
            uri,
            strict: Some(strict),
        }
    }

    /// Its transitional URI (what a document written here declares).
    #[must_use]
    pub const fn uri(self) -> &'static str {
        self.uri
    }

    /// Whether `namespace` (a name's) is this one.
    #[must_use]
    pub fn holds(self, namespace: Option<&str>) -> bool {
        namespace.is_some_and(|n| n == self.uri || Some(n) == self.strict)
    }

    /// Whether `element`'s name is `local` in this namespace.
    #[must_use]
    pub fn named(self, element: &Element, local: &str) -> bool {
        element.name().local() == local && self.holds(element.name().namespace())
    }
}

/// The first child element named `name` in `ns`.
pub fn child_in<'a>(element: &'a Element, ns: Ns, name: &str) -> Option<&'a Element> {
    element.child_elements().find(|e| ns.named(e, name)).map(|e| &**e)
}

/// Every child element named `name` in `ns`, in order.
pub fn children_in<'a>(element: &'a Element, ns: Ns, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
    element
        .child_elements()
        .filter(move |e| ns.named(e, name))
        .map(|e| &**e)
}

/// An attribute named `name` in `ns`, or (`None`) in no namespace -- WordprocessingML's
/// attributes are in its namespace (`w:val`), DrawingML's in none (`cx`).
pub fn attr_in<'a>(element: &'a Element, ns: Option<Ns>, name: &str) -> Option<&'a str> {
    element
        .attributes()
        .iter()
        .find(|a| {
            a.name.local() == name
                && match ns {
                    Some(ns) => ns.holds(a.name.namespace()),
                    None => a.name.namespace().is_none(),
                }
        })
        .map(|a| &*a.value)
}

/// [`attr_in`] read as an integer ([`py_int`]).
pub fn attr_int_in(element: &Element, ns: Option<Ns>, name: &str) -> Option<i64> {
    attr_in(element, ns, name).and_then(py_int)
}

/// [`attr_in`] read as a measure in twips ([`twips`]).
pub fn attr_twips_in(element: &Element, ns: Option<Ns>, name: &str) -> Option<i64> {
    attr_in(element, ns, name).and_then(twips)
}

/// A measure in twips as Word reads one (`ST_TwipsMeasure`, `ST_SignedTwipsMeasure`): an
/// integer ([`py_int`]), or a universal measure -- a decimal number and a unit, `pt`, `pc`
/// or `pi` (a pica, 12 points), `in`, `mm` or `cm` -- which ISO/IEC 29500 Strict writes
/// (`35.40pt`), in twips (a twentieth of a point) cut to a whole twip toward zero, as Word
/// 16.106 reads one (measured: `10.05mm` is 569 twips, `12.33pt` 246;
/// `crates/ooxml-oracle/word/observations/strict-word.json`). `None` for anything else.
#[must_use]
pub fn twips(text: &str) -> Option<i64> {
    if let Some(value) = py_int(text) {
        return Some(value);
    }
    let unit_at = text.len().checked_sub(2)?;
    let (number, unit) = (text.get(..unit_at)?, text.get(unit_at..)?);
    let per_unit = match unit {
        "pt" => 20.0,
        "pc" | "pi" => 240.0,
        "in" => 1440.0,
        "mm" => 1440.0 / 25.4,
        "cm" => 1440.0 / 2.54,
        _ => return None,
    };
    let digits = number.strip_prefix('-').unwrap_or(number);
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, "0"));
    let decimal = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !decimal(whole) || !decimal(fraction) {
        return None;
    }
    let value: f64 = number.parse().ok()?;
    let twips = (value * per_unit).trunc();
    #[allow(clippy::cast_possible_truncation)]
    (twips.is_finite() && twips.abs() < 9.0e15).then_some(twips as i64)
}

/// [`attr_in`] read as an `ST_OnOff` value: absent is on ([`attr_on`]).
pub fn attr_on_in(element: &Element, ns: Option<Ns>, name: &str) -> bool {
    attr_in(element, ns, name).is_none_or(is_on)
}

/// [`attr_in`] read as a DrawingML boolean ([`attr_flag`]).
pub fn attr_flag_in(element: &Element, ns: Option<Ns>, name: &str) -> Option<bool> {
    attr_in(element, ns, name).map(|v| !matches!(v, "0" | "false" | "off"))
}

/// An element's local name.
pub fn local(element: &Element) -> &str {
    element.name().local()
}

/// Every child element, in order (text, comments and processing instructions skipped).
pub fn elements(element: &Element) -> impl Iterator<Item = &Arc<Element>> {
    element.child_elements()
}

/// Every child element with local name `name`, in order.
pub fn children<'a>(element: &'a Element, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
    element.child_elements().filter(move |e| local(e) == name).map(|e| &**e)
}

/// The first child element with local name `name`.
pub fn child<'a>(element: &'a Element, name: &str) -> Option<&'a Element> {
    element.child_elements().find(|e| local(e) == name).map(|e| &**e)
}

/// An attribute by local name: one without a namespace first, then the first in document
/// order whose local name is `name` (ElementTree's `get` then a scan, as docx2svg reads).
pub fn attr<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    let attributes = element.attributes();
    attributes
        .iter()
        .find(|a| a.name.namespace().is_none() && a.name.local() == name)
        .or_else(|| attributes.iter().find(|a| a.name.local() == name))
        .map(|a| &*a.value)
}

/// An attribute in exactly namespace `namespace`.
pub fn attr_ns<'a>(element: &'a Element, namespace: &str, name: &str) -> Option<&'a str> {
    element.attribute(Some(namespace), name)
}

/// An integer as Python's `int()` reads a string: surrounding whitespace, an optional
/// sign, ASCII digits with single underscores between them. `None` for anything else or
/// a value outside `i64`.
pub fn py_int(text: &str) -> Option<i64> {
    let s = text.trim_matches(|c: char| c.is_whitespace());
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, s.get(1..)?),
        Some(b'+') => (false, s.get(1..)?),
        _ => (false, s),
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    let mut value: i64 = 0;
    for c in digits.chars() {
        if c == '_' {
            continue;
        }
        let d = i64::from(c.to_digit(10)?);
        value = value.checked_mul(10)?.checked_add(d)?;
    }
    Some(if negative { -value } else { value })
}

/// An integer attribute (docx2svg's `attr_int`): `None` where absent or not an integer.
pub fn attr_int(element: &Element, name: &str) -> Option<i64> {
    attr(element, name).and_then(py_int)
}

/// An `ST_OnOff` attribute (docx2svg's `attr_bool`): absent is on; else on for `1`,
/// `true`, `on` in any case.
pub fn attr_on(element: &Element, name: &str) -> bool {
    attr(element, name).is_none_or(is_on)
}

/// Whether an `ST_OnOff` value is on: `1`, `true` or `on`, in any case.
pub fn is_on(value: &str) -> bool {
    matches!(value.to_lowercase().as_str(), "1" | "true" | "on")
}

/// A DrawingML boolean as docx2svg reads one: `None` where absent, else whether the value
/// is not `0`, `false` or `off` (as written).
pub fn attr_flag(element: &Element, name: &str) -> Option<bool> {
    attr(element, name).map(|v| !matches!(v, "0" | "false" | "off"))
}

/// An element's text as ElementTree's `.text` holds it: the text before its first child
/// element (comments and processing instructions, which ElementTree drops, skipped).
pub fn text(element: &Element) -> String {
    let mut out = String::new();
    for node in element.children() {
        match node {
            Node::Text(t) => out.push_str(t),
            Node::Element(_) => break,
            _ => {}
        }
    }
    out
}

/// `text`, or `None` where the element holds none (ElementTree's `None`).
pub fn text_opt(element: &Element) -> Option<String> {
    let t = text(element);
    (!t.is_empty()).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::{twips, Ns};

    #[test]
    fn a_universal_measure_is_read_in_twips_as_word_reads_it() {
        // Word 16.106 read each `w:pgMar` value written so and saved it in twips
        // (crates/ooxml-oracle/word/observations/strict-word.json, "units")
        let measured = [
            ("1in", 1440),
            ("25.4mm", 1440),
            ("2.54cm", 1440),
            ("6pc", 1440),
            ("3pi", 720),
            ("35.4pt", 708),
            ("0.5in", 720),
            ("1.27cm", 720),
            ("12.7mm", 720),
            ("0.75in", 1080),
            ("17.95pt", 359),
            ("1.5pc", 360),
            ("2.25pi", 540),
            ("1mm", 56),
            ("1.001in", 1441),
            ("10.05mm", 569),
            ("3.333cm", 1889),
            ("12.33pt", 246),
            ("0.1pc", 24),
            ("7.7pi", 1848),
            ("0.03in", 43),
        ];
        for (written, word) in measured {
            assert_eq!(twips(written), Some(word), "{written}");
        }
    }

    #[test]
    fn an_integer_is_read_as_before_and_anything_else_is_not_a_measure() {
        assert_eq!(twips("708"), Some(708));
        assert_eq!(twips(" -12 "), Some(-12));
        assert_eq!(twips("-12.5pt"), Some(-250));
        for not in ["", "pt", "1.pt", ".5pt", "1px", "1 pt", "1e2pt", "1,5pt", "12%"] {
            assert_eq!(twips(not), None, "{not:?}");
        }
    }

    #[test]
    fn a_vocabulary_holds_its_strict_uri_beside_its_transitional_one() {
        assert!(Ns::W.holds(Some("http://schemas.openxmlformats.org/wordprocessingml/2006/main")));
        assert!(Ns::W.holds(Some("http://purl.oclc.org/ooxml/wordprocessingml/main")));
        assert!(!Ns::W.holds(Some("http://schemas.microsoft.com/office/word/2010/wordml")));
        assert!(!Ns::W.holds(None));
        assert!(Ns::W14.holds(Some("http://schemas.microsoft.com/office/word/2010/wordml")));
    }
}
