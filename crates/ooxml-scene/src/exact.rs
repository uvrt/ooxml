//! Owns: a page's display list for the Word layout's paint -- SVG elements ([`Element`])
//! whose lengths are exact decimals ([`Decimal4`]: four decimals, rounded by the producer
//! from an exact value) and whose attributes, live-view ids (`data-*`) among them, keep the
//! order their producer gave -- and the writer that writes one as SVG text ([`write`]):
//! each element of a block on its own line, an inline element's children run together, as
//! docx2svg writes a page.
//!
//! The order is kept because the reference output (docx2svg's string-built SVG) fixes it,
//! and a byte-for-byte equal writer must reproduce it; SVG gives it no meaning.
//!
//! Must not: know any OOXML, round a length (its producer has), or parse SVG.

use std::fmt;

/// A length to four decimals: ten-thousandths of a unit, already rounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Decimal4(pub i64);

impl Decimal4 {
    /// A whole number.
    pub fn whole(value: i64) -> Decimal4 {
        Decimal4(value.saturating_mul(10_000))
    }
}

impl fmt::Display for Decimal4 {
    /// No trailing zeros and no point for a whole number (docx2svg's `svg.number`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let magnitude = self.0.unsigned_abs();
        let (whole, fraction) = (magnitude / 10_000, magnitude % 10_000);
        if fraction == 0 {
            return write!(f, "{sign}{whole}");
        }
        let digits = format!("{fraction:04}");
        write!(f, "{sign}{whole}.{}", digits.trim_end_matches('0'))
    }
}

/// An attribute's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A whole number.
    Int(i64),
    /// A length.
    Number(Decimal4),
    /// Lengths separated by spaces.
    Numbers(Vec<Decimal4>),
    /// Text, quoted as the reference quotes an attribute (`quoteattr`).
    Text(String),
    /// Text written between double quotes as it is: the producer says it needs no
    /// escaping (a keyword, a colour, a dash list).
    Plain(String),
    /// Attributes a painter wrote, `name="value"` pairs written as they are, in its place
    /// among the element's (its name is not written).
    Attributes(String),
    /// Bytes as a `data:` URI of their media type, in base64.
    Data {
        /// The media type.
        mime: String,
        /// The bytes.
        bytes: std::sync::Arc<[u8]>,
    },
}

/// How an element's children are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Each child on a line of its own, the closing tag on its own line.
    Block,
    /// The children run together on the element's line.
    Inline,
}

/// One element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Its name.
    pub tag: &'static str,
    /// Its attributes, in order.
    pub attrs: Vec<(&'static str, Value)>,
    /// Its children.
    pub children: Vec<Element>,
    /// How they are written.
    pub flow: Flow,
    /// Its text content (escaped when written).
    pub text: Option<String>,
    /// Written with a closing tag even when empty (a group that may hold nothing).
    pub paired: bool,
}

impl Element {
    /// An empty element.
    pub fn new(tag: &'static str) -> Element {
        Element {
            tag,
            attrs: Vec::new(),
            children: Vec::new(),
            flow: Flow::Block,
            text: None,
            paired: false,
        }
    }

    /// Markup its producer wrote, written as it is (a painter's that has not moved onto
    /// the display list yet).
    pub fn raw(markup: String) -> Element {
        let mut e = Element::new("");
        e.text = Some(markup);
        e
    }

    /// Written with a closing tag even when it holds nothing.
    #[must_use]
    pub fn paired(mut self) -> Element {
        self.paired = true;
        self
    }

    /// With an attribute added.
    #[must_use]
    pub fn attr(mut self, name: &'static str, value: Value) -> Element {
        self.attrs.push((name, value));
        self
    }

    /// With children written inline.
    #[must_use]
    pub fn inline(mut self) -> Element {
        self.flow = Flow::Inline;
        self
    }

    /// With text content.
    #[must_use]
    pub fn content(mut self, text: String) -> Element {
        self.text = Some(text);
        self
    }
}

/// `text` escaped as element content (`&`, `<`, `>`).
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// `text` quoted as an attribute's value (Python's `quoteattr`): escaped as content, line
/// ends and tabs as character references, between double quotes -- single ones where it
/// holds a double quote and no single one, else its double quotes escaped too.
pub fn quote(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() + 2);
    for ch in escape(text).chars() {
        match ch {
            '\n' => escaped.push_str("&#10;"),
            '\r' => escaped.push_str("&#13;"),
            '\t' => escaped.push_str("&#9;"),
            _ => escaped.push(ch),
        }
    }
    if escaped.contains('"') {
        if escaped.contains('\'') {
            return format!("\"{}\"", escaped.replace('"', "&quot;"));
        }
        return format!("'{escaped}'");
    }
    format!("\"{escaped}\"")
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Int(n) => {
            out.push('"');
            out.push_str(&n.to_string());
            out.push('"');
        }
        Value::Number(n) => {
            out.push('"');
            out.push_str(&n.to_string());
            out.push('"');
        }
        Value::Numbers(ns) => {
            out.push('"');
            let mut first = true;
            for n in ns {
                if !first {
                    out.push(' ');
                }
                first = false;
                out.push_str(&n.to_string());
            }
            out.push('"');
        }
        Value::Text(t) => out.push_str(&quote(t)),
        Value::Plain(t) | Value::Attributes(t) => {
            out.push('"');
            out.push_str(t);
            out.push('"');
        }
        Value::Data { mime, bytes } => {
            out.push_str("\"data:");
            out.push_str(mime);
            out.push_str(";base64,");
            crate::base64::encode_into(bytes, out);
            out.push('"');
        }
    }
}

fn open_tag(out: &mut String, e: &Element) {
    out.push('<');
    out.push_str(e.tag);
    for (name, value) in &e.attrs {
        out.push(' ');
        if let Value::Attributes(written) = value {
            out.push_str(written);
            continue;
        }
        out.push_str(name);
        out.push('=');
        write_value(out, value);
    }
}

fn write_element(out: &mut String, e: &Element, flow: Flow) {
    if e.tag.is_empty() {
        out.push_str(e.text.as_deref().unwrap_or_default());
        return;
    }
    open_tag(out, e);
    if e.children.is_empty() && e.text.is_none() && !e.paired {
        out.push_str("/>");
        return;
    }
    out.push('>');
    if let Some(text) = &e.text {
        out.push_str(&escape(text));
    }
    let inner = if flow == Flow::Inline { Flow::Inline } else { e.flow };
    for child in &e.children {
        if inner == Flow::Block {
            out.push('\n');
        }
        write_element(out, child, inner);
    }
    if inner == Flow::Block && (!e.children.is_empty() || (e.paired && e.text.is_none())) {
        out.push('\n');
    }
    out.push_str("</");
    out.push_str(e.tag);
    out.push('>');
}

/// `elements` as a block element writes its children: each on a line of its own, a line
/// end before each (a group's content written apart from the group).
pub fn write_lines(elements: &[Element]) -> String {
    let mut out = String::new();
    for e in elements {
        out.push('\n');
        write_element(&mut out, e, Flow::Block);
    }
    out
}

/// `root` as an SVG document's text: its children each on a line of their own, ending with
/// a line end.
pub fn write(root: &Element) -> String {
    let mut out = String::new();
    write_element(&mut out, root, Flow::Block);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_lose_their_trailing_zeros() {
        assert_eq!(Decimal4(3_355_000).to_string(), "335.5");
        assert_eq!(Decimal4(3_000_000).to_string(), "300");
        assert_eq!(Decimal4(-12_345).to_string(), "-1.2345");
        assert_eq!(Decimal4(5).to_string(), "0.0005");
    }

    #[test]
    fn attributes_are_quoted_as_python_quotes_them() {
        assert_eq!(quote("w:body/w:p[1]"), "\"w:body/w:p[1]\"");
        assert_eq!(quote("a \"b\""), "'a \"b\"'");
        assert_eq!(quote("a \"b\" 'c'"), "\"a &quot;b&quot; 'c'\"");
        assert_eq!(quote("x<y&z"), "\"x&lt;y&amp;z\"");
    }

    #[test]
    fn blocks_put_children_on_lines_and_inline_groups_run_together() {
        let line = Element::new("g").attr("data-docx-line", Value::Int(0)).child_text();
        let root = Element::new("svg").with(line).with(
            Element::new("g")
                .inline()
                .with(Element::new("rect").attr("x", Value::Number(Decimal4::whole(1)))),
        );
        assert_eq!(
            write(&root),
            "<svg>\n<g data-docx-line=\"0\">\n<text>a&amp;b</text>\n</g>\n<g><rect x=\"1\"/></g>\n</svg>\n"
        );
    }

    impl Element {
        fn with(mut self, child: Element) -> Element {
            self.children.push(child);
            self
        }

        fn child_text(self) -> Element {
            self.with(Element::new("text").content("a&b".into()))
        }
    }
}
