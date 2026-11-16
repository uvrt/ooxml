//! Owns: character data -- text runs with end-of-line normalisation, CDATA appended as
//! text, character and predefined entity references, entity references kept opaque where
//! libxml2 keeps them, and attribute values with their normalisation (tab, line feed and
//! carriage return as a space; a reference's character kept as it is).
//!
//! Must not: expand a declared or external entity.

use super::Parser;
use crate::node::Node;
use crate::XmlError;

/// Text read but not yet made a node: nothing, one slice of the input (the common case,
/// copied once), or text assembled from several pieces.
#[derive(Default)]
pub(super) enum TextBuf<'a> {
    #[default]
    Nothing,
    Slice(&'a str),
    Owned(String),
}

impl<'a> TextBuf<'a> {
    fn len(&self) -> usize {
        match self {
            TextBuf::Nothing => 0,
            TextBuf::Slice(s) => s.len(),
            TextBuf::Owned(s) => s.len(),
        }
    }

    /// Append to an owned copy (made on first need).
    fn with_owned(&mut self, f: impl FnOnce(&mut String)) {
        let mut owned = match std::mem::take(self) {
            TextBuf::Nothing => String::new(),
            TextBuf::Slice(s) => s.to_owned(),
            TextBuf::Owned(s) => s,
        };
        f(&mut owned);
        *self = TextBuf::Owned(owned);
    }

    fn push_str(&mut self, s: &'a str) {
        match self {
            TextBuf::Nothing => *self = TextBuf::Slice(s),
            _ => self.with_owned(|o| o.push_str(s)),
        }
    }

    fn push(&mut self, c: char) {
        self.with_owned(|o| o.push(c));
    }
}

/// The character a reference names, if it is one XML allows.
fn char_ref(body: &str) -> Option<char> {
    let value = match body.strip_prefix('x') {
        Some(hex) if !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit()) => u32::from_str_radix(hex, 16).ok()?,
        Some(_) => return None,
        None if !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit()) => body.parse().ok()?,
        None => return None,
    };
    let allowed = matches!(value, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF);
    allowed.then(|| char::from_u32(value)).flatten()
}

fn predefined(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

/// A reference after its `&`.
enum Reference<'a> {
    Char(char),
    Entity(&'a str),
}

impl<'a> Parser<'a> {
    /// The text read so far, as a node.
    pub(super) fn flush(&mut self, text: &mut TextBuf<'a>, kids: &mut Vec<Node>) -> Result<(), XmlError> {
        let atom = match std::mem::take(text) {
            TextBuf::Nothing => return Ok(()),
            TextBuf::Slice(s) => self.interner.atom(s),
            TextBuf::Owned(s) => self.interner.atom(&s),
        };
        self.count_node()?;
        kids.push(Node::Text(atom));
        Ok(())
    }

    fn check_text_len(&self, text: &TextBuf<'a>) -> Result<(), XmlError> {
        if text.len() > self.limits.max_text_len {
            return Err(XmlError::Limit { what: "text node length", limit: self.limits.max_text_len });
        }
        Ok(())
    }

    /// Append raw character data (a text run or a CDATA section), end-of-line normalised.
    pub(super) fn append_text(&self, text: &mut TextBuf<'a>, raw: &'a str) -> Result<(), XmlError> {
        let mut rest = raw;
        while let Some(cr) = rest.find('\r') {
            text.push_str(rest.get(..cr).unwrap_or_default());
            text.push('\n');
            rest = rest.get(cr + 1..).unwrap_or_default();
            rest = rest.strip_prefix('\n').unwrap_or(rest);
        }
        if !rest.is_empty() {
            text.push_str(rest);
        }
        self.check_text_len(text)
    }

    /// Character data up to the next `<` or `&`.
    pub(super) fn text_run(&mut self, text: &mut TextBuf<'a>) -> Result<(), XmlError> {
        let start = self.i;
        let rest = self.s.get(start..).unwrap_or_default();
        let len = crate::encoding::find_either(rest, b'<', b'&');
        let run = self.slice(start, start + len);
        if let Some(at) = run.find("]]>") {
            return Err(XmlError::Syntax { offset: start + at, message: "']]>' is not allowed in content" });
        }
        self.i = start + len;
        self.append_text(text, run)
    }

    /// `&#...;` or `&name;` (the `&` at the current position).
    fn reference(&mut self) -> Result<Reference<'a>, XmlError> {
        let at = self.i;
        self.i += 1;
        let body = self.until(b";", "a reference was not terminated")?;
        if let Some(number) = body.strip_prefix('#') {
            return char_ref(number)
                .map(Reference::Char)
                .ok_or(XmlError::Syntax { offset: at, message: "a character reference to a character XML does not allow" });
        }
        if super::chars::name_len(body) != body.len() || body.is_empty() {
            return Err(XmlError::Syntax { offset: at, message: "an entity reference is not a name" });
        }
        Ok(predefined(body).map_or(Reference::Entity(body), Reference::Char))
    }

    pub(super) fn reference_in_content(&mut self, text: &mut TextBuf<'a>, kids: &mut Vec<Node>) -> Result<(), XmlError> {
        let at = self.i;
        match self.reference()? {
            Reference::Char(c) => {
                text.push(c);
                self.check_text_len(text)
            }
            Reference::Entity(name) if self.keep_entities => {
                self.flush(text, kids)?;
                self.count_node()?;
                let name = self.interner.atom(name);
                kids.push(Node::EntityRef(name));
                Ok(())
            }
            Reference::Entity(name) => Err(XmlError::UndefinedEntity { name: name.to_owned(), offset: at }),
        }
    }

    /// A quoted attribute value, decoded and normalised.
    pub(super) fn attribute_value(&mut self) -> Result<crate::name::Atom, XmlError> {
        let quote = match self.peek() {
            Some(q @ (b'"' | b'\'')) => q,
            _ => return Err(self.err("an attribute value must be quoted")),
        };
        self.i += 1;
        let start = self.i;
        let rest = self.s.get(start..).unwrap_or_default();
        let len = rest.iter().position(|&b| b == quote).ok_or_else(|| self.err("an attribute value was not closed"))?;
        let raw = self.slice(start, start + len);
        if let Some(at) = raw.find('<') {
            return Err(XmlError::Syntax { offset: start + at, message: "'<' is not allowed in an attribute value" });
        }
        let value = if raw.bytes().any(|b| matches!(b, b'&' | b'\t' | b'\n' | b'\r')) {
            let v = self.normalised_value(raw, start)?;
            self.interner.atom(&v)
        } else {
            self.interner.atom(raw)
        };
        if value.len() > self.limits.max_attribute_len {
            return Err(XmlError::Limit { what: "attribute value length", limit: self.limits.max_attribute_len });
        }
        self.i = start + len + 1;
        Ok(value)
    }

    fn normalised_value(&mut self, raw: &'a str, start: usize) -> Result<String, XmlError> {
        let mut out = String::with_capacity(raw.len());
        let mut chars = raw.char_indices().peekable();
        while let Some((k, c)) = chars.next() {
            match c {
                '\r' => {
                    chars.next_if(|&(_, n)| n == '\n');
                    out.push(' ');
                }
                '\t' | '\n' => out.push(' '),
                '&' => {
                    self.i = start + k;
                    let end = raw.get(k..).and_then(|r| r.find(';')).map_or(raw.len(), |e| k + e);
                    match self.reference()? {
                        Reference::Char(c) => out.push(c),
                        Reference::Entity(_) if self.keep_entities => {
                            return Err(XmlError::Unsupported { what: "an entity reference in an attribute value", offset: start + k })
                        }
                        Reference::Entity(name) => {
                            return Err(XmlError::UndefinedEntity { name: name.to_owned(), offset: start + k })
                        }
                    }
                    while chars.next_if(|&(j, _)| j <= end).is_some() {}
                }
                c => out.push(c),
            }
        }
        Ok(out)
    }
}
