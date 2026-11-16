//! Owns: reading a part into a [`Tree`] as libxml2 reads it with lxml's settings in
//! `ooxml_edit.xml.parse_xml` (`resolve_entities=False`, no `huge_tree`): end-of-line and
//! attribute-value normalisation, character and predefined references decoded, CDATA as
//! text, whitespace outside the root dropped, entities never expanded, and libxml2's
//! well-formedness and namespace errors (an undeclared prefix, a duplicate attribute, `--`
//! in a comment, `]]>` in text, a character XML does not allow).
//!
//! The document level lives here; elements and their content in `content`, text and
//! references in `text`, sharing in `intern`, character classes in `chars`.
//!
//! Must not: expand an entity, fetch anything, or accept input beyond [`XmlLimits`].

mod chars;
mod content;
mod intern;
mod text;

use crate::encoding;
use crate::name::Atom;
use crate::node::{Doctype, ExternalId, Node, Pi, Tree};
use crate::XmlError;
use chars::is_space;
use intern::Interner;
use ooxml_base::XmlLimits;
use std::borrow::Cow;
use std::sync::Arc;

/// Parse a part (UTF-8, UTF-16, US-ASCII or ISO-8859-1, as its byte-order mark or
/// declaration says) within `limits`.
pub fn parse(input: &[u8], limits: &XmlLimits) -> Result<Tree, XmlError> {
    let text = encoding::decode(input)?;
    if let Some(offset) = encoding::first_invalid_char(&text) {
        return Err(XmlError::Syntax {
            offset,
            message: "a character XML does not allow",
        });
    }
    Parser::new(&text, limits).document()
}

/// A namespace binding in scope.
struct Binding {
    prefix: Option<Atom>,
    uri: Atom,
}

pub(crate) struct Parser<'a> {
    src: &'a str,
    s: &'a [u8],
    i: usize,
    limits: &'a XmlLimits,
    interner: Interner,
    nodes: usize,
    scope: Vec<Binding>,
    scope_id: usize,
    scopes_made: usize,
    standalone: bool,
    /// Whether an undeclared entity may stay a reference (an external DTD, not standalone).
    keep_entities: bool,
    tag_namespaces: Vec<crate::node::NamespaceDecl>,
    tag_attributes: Vec<(&'a str, usize, Atom)>,
    tag_resolved: Vec<crate::node::Attribute>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, limits: &'a XmlLimits) -> Self {
        Parser {
            src,
            s: src.as_bytes(),
            i: 0,
            limits,
            interner: Interner::default(),
            nodes: 0,
            scope: Vec::new(),
            scope_id: 0,
            scopes_made: 0,
            standalone: false,
            keep_entities: false,
            tag_namespaces: Vec::new(),
            tag_attributes: Vec::new(),
            tag_resolved: Vec::new(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.s.get(self.i + ahead).copied()
    }

    fn starts(&self, prefix: &[u8]) -> bool {
        self.s.get(self.i..).is_some_and(|rest| rest.starts_with(prefix))
    }

    /// The text from `a` to `b` (both on character boundaries by construction).
    fn slice(&self, a: usize, b: usize) -> &'a str {
        self.src.get(a..b).unwrap_or_default()
    }

    fn err(&self, message: &'static str) -> XmlError {
        XmlError::Syntax {
            offset: self.i,
            message,
        }
    }

    /// Skip whitespace; whether there was any.
    fn skip_space(&mut self) -> bool {
        let start = self.i;
        while self.peek().is_some_and(is_space) {
            self.i += 1;
        }
        self.i > start
    }

    fn expect(&mut self, b: u8, message: &'static str) -> Result<(), XmlError> {
        if self.peek() == Some(b) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(message))
        }
    }

    /// Advance past the next `delimiter`, returning what came before it.
    fn until(&mut self, delimiter: &[u8], message: &'static str) -> Result<&'a str, XmlError> {
        let rest = self.s.get(self.i..).unwrap_or_default();
        let at = rest
            .windows(delimiter.len())
            .position(|w| w == delimiter)
            .ok_or_else(|| self.err(message))?;
        let found = self.slice(self.i, self.i + at);
        self.i += at + delimiter.len();
        Ok(found)
    }

    /// A name at the current position.
    fn name(&mut self, message: &'static str) -> Result<&'a str, XmlError> {
        let len = chars::name_len(self.slice(self.i, self.s.len()));
        if len == 0 {
            return Err(self.err(message));
        }
        if len > self.limits.max_name_len {
            return Err(XmlError::Limit {
                what: "name length",
                limit: self.limits.max_name_len,
            });
        }
        let name = self.slice(self.i, self.i + len);
        self.i += len;
        Ok(name)
    }

    fn count_node(&mut self) -> Result<(), XmlError> {
        self.nodes += 1;
        if self.nodes > self.limits.max_nodes {
            return Err(XmlError::Limit {
                what: "nodes in a part",
                limit: self.limits.max_nodes,
            });
        }
        Ok(())
    }

    fn document(mut self) -> Result<Tree, XmlError> {
        if self.starts(b"<?xml") && self.peek_at(5).is_some_and(is_space) {
            self.declaration()?;
        }
        let mut prolog = Vec::new();
        let mut doctype = None;
        loop {
            self.skip_space();
            if self.starts(b"<!DOCTYPE") {
                if doctype.is_some() {
                    return Err(self.err("a second document type declaration"));
                }
                doctype = Some(self.doctype()?);
            } else if !self.misc(&mut prolog)? {
                break;
            }
        }
        match self.peek() {
            Some(b'<') => {}
            None => return Err(self.err("the document is empty")),
            Some(_) => return Err(self.err("content before the root element")),
        }
        let root = self.root_element()?;
        let mut epilog = Vec::new();
        loop {
            self.skip_space();
            if self.peek().is_none() {
                break;
            }
            if !self.misc(&mut epilog)? {
                return Err(self.err("extra content at the end of the document"));
            }
        }
        Ok(Tree::new(doctype, prolog, root, epilog))
    }

    /// A comment or processing instruction outside the root; `false` when neither is next.
    fn misc(&mut self, out: &mut Vec<Node>) -> Result<bool, XmlError> {
        if self.starts(b"<!--") {
            let c = self.comment()?;
            out.push(c);
        } else if self.starts(b"<?") {
            let p = self.pi()?;
            out.push(p);
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    /// `<?xml version=... encoding=... standalone=...?>`: only `standalone` matters after
    /// decoding (the writer always writes its own declaration).
    fn declaration(&mut self) -> Result<(), XmlError> {
        self.i += 5;
        let mut seen_version = false;
        loop {
            let spaced = self.skip_space();
            if self.starts(b"?>") {
                self.i += 2;
                break;
            }
            if !spaced {
                return Err(self.err("malformed XML declaration"));
            }
            let key = self.name("malformed XML declaration")?;
            self.skip_space();
            self.expect(b'=', "malformed XML declaration")?;
            self.skip_space();
            let value = self.literal()?;
            match key {
                "version" if !seen_version && value.starts_with("1.") => seen_version = true,
                "encoding" if seen_version => {}
                "standalone" if seen_version && (value == "yes" || value == "no") => self.standalone = value == "yes",
                _ => return Err(self.err("malformed XML declaration")),
            }
        }
        if seen_version {
            Ok(())
        } else {
            Err(self.err("the XML declaration has no version"))
        }
    }

    /// A quoted literal (no references; DTD and declaration values).
    fn literal(&mut self) -> Result<&'a str, XmlError> {
        let quote = match self.peek() {
            Some(q @ (b'"' | b'\'')) => q,
            _ => return Err(self.err("a quoted value was expected")),
        };
        self.i += 1;
        self.until(&[quote], "an unterminated quoted value")
    }

    /// `<!DOCTYPE name (SYSTEM "s" | PUBLIC "p" "s")? >`; an internal subset is refused.
    fn doctype(&mut self) -> Result<Doctype, XmlError> {
        self.i += 9;
        if !self.skip_space() {
            return Err(self.err("space required after DOCTYPE"));
        }
        let name = self.name("a DOCTYPE name was expected")?;
        let spaced = self.skip_space();
        let external = if spaced && self.starts(b"SYSTEM") {
            self.i += 6;
            self.require_space()?;
            Some(ExternalId::System(self.literal()?.into()))
        } else if spaced && self.starts(b"PUBLIC") {
            self.i += 6;
            self.require_space()?;
            let public = self.literal()?;
            self.require_space()?;
            Some(ExternalId::Public(public.into(), self.literal()?.into()))
        } else {
            None
        };
        self.skip_space();
        if self.peek() == Some(b'[') {
            return Err(XmlError::Unsupported {
                what: "an internal DTD subset",
                offset: self.i,
            });
        }
        self.expect(b'>', "DOCTYPE improperly terminated")?;
        self.keep_entities = external.is_some() && !self.standalone;
        Ok(Doctype {
            name: name.into(),
            external,
        })
    }

    fn require_space(&mut self) -> Result<(), XmlError> {
        if self.skip_space() {
            Ok(())
        } else {
            Err(self.err("space required"))
        }
    }

    /// `<!-- ... -->`, with no `--` inside and no `-` last.
    fn comment(&mut self) -> Result<Node, XmlError> {
        self.i += 4;
        let start = self.i;
        let body = self.until(b"-->", "an unterminated comment")?;
        if body.contains("--") || body.ends_with('-') {
            return Err(XmlError::Syntax {
                offset: start,
                message: "double hyphen within a comment",
            });
        }
        self.count_node()?;
        Ok(Node::Comment(self.interner.atom(&normalise_eol(body))))
    }

    /// `<?target data?>`: the whitespace after the target dropped, the data as written.
    fn pi(&mut self) -> Result<Node, XmlError> {
        self.i += 2;
        let target = self.name("a processing instruction's target was expected")?;
        if target.eq_ignore_ascii_case("xml") {
            return Err(self.err("an XML declaration is allowed only at the start of the document"));
        }
        let data = if self.starts(b"?>") {
            self.i += 2;
            ""
        } else {
            self.require_space()?;
            self.skip_space();
            self.until(b"?>", "an unterminated processing instruction")?
        };
        self.count_node()?;
        let data = self.interner.atom(&normalise_eol(data));
        Ok(Node::Pi(Arc::new(Pi {
            target: self.interner.atom(target),
            data,
        })))
    }
}

/// `\r\n` and `\r` as `\n` (XML 1.0 section 2.11).
fn normalise_eol(s: &str) -> Cow<'_, str> {
    if s.contains('\r') {
        Cow::Owned(s.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(s)
    }
}
