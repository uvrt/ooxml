//! Owns: elements -- start tags with their namespace declarations and attributes, the
//! content loop (an explicit stack, so depth costs no native stack), end tags, and
//! namespace resolution.
//!
//! Must not: decode text or references (`text` does), or recurse.

use super::chars::is_qname;
use super::text::TextBuf;
use super::{Binding, Parser};
use crate::name::{Atom, Name};
use crate::node::{Attribute, Element, Head, NamespaceDecl, Node};
use crate::XmlError;
use std::collections::HashSet;
use std::sync::Arc;

const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// An element whose end tag has not been read.
struct Open<'a> {
    head: Arc<Head>,
    raw: &'a str,
    /// Where its children start in the shared child buffer.
    start: usize,
    scope_len: usize,
    scope_id: usize,
}

/// The offset of the `>` that ends the start tag at `start` (quoted values skipped), or
/// `None` when a `<` or the end of input comes first (the full parse then says why).
fn tag_end(s: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    loop {
        match s.get(i)? {
            b'>' => return Some(i),
            b'<' => return None,
            &q @ (b'"' | b'\'') => {
                let rest = s.get(i + 1..)?;
                i += 2 + rest.iter().position(|&c| c == q)?;
            }
            _ => i += 1,
        }
    }
}

/// A start tag as read.
struct StartTag<'a> {
    head: Arc<Head>,
    raw: &'a str,
    empty: bool,
    scope_len: usize,
    scope_id: usize,
}

impl<'a> Parser<'a> {
    /// The root element and everything in it.
    pub(super) fn root_element(&mut self) -> Result<Arc<Element>, XmlError> {
        let mut stack: Vec<Open<'a>> = Vec::new();
        let mut kids: Vec<Node> = Vec::new();
        let mut text = TextBuf::default();
        let mut tag = self.start_tag(0)?;
        loop {
            if tag.empty {
                self.restore_scope(tag.scope_len, tag.scope_id);
                let at = kids.len();
                let element = self.interner.element(tag.head, &mut kids, at);
                if stack.is_empty() {
                    return Ok(element);
                }
                kids.push(Node::Element(element));
            } else {
                let start = kids.len();
                stack.push(Open {
                    head: tag.head,
                    raw: tag.raw,
                    start,
                    scope_len: tag.scope_len,
                    scope_id: tag.scope_id,
                });
            }
            // content up to the next start tag, closing elements on the way
            loop {
                match (self.peek(), self.peek_at(1)) {
                    (None, _) => return Err(self.err("premature end of data")),
                    (Some(b'<'), Some(b'/')) => {
                        self.flush(&mut text, &mut kids)?;
                        let open = stack.pop().ok_or_else(|| self.err("an end tag without a start tag"))?;
                        self.end_tag(open.raw)?;
                        self.restore_scope(open.scope_len, open.scope_id);
                        let element = self.interner.element(open.head, &mut kids, open.start);
                        if stack.is_empty() {
                            return Ok(element);
                        }
                        kids.push(Node::Element(element));
                    }
                    (Some(b'<'), Some(b'!')) => self.markup_in_content(&mut text, &mut kids)?,
                    (Some(b'<'), Some(b'?')) => {
                        self.flush(&mut text, &mut kids)?;
                        let pi = self.pi()?;
                        kids.push(pi);
                    }
                    (Some(b'<'), _) => {
                        self.flush(&mut text, &mut kids)?;
                        tag = self.start_tag(stack.len())?;
                        break;
                    }
                    (Some(b'&'), _) => self.reference_in_content(&mut text, &mut kids)?,
                    (Some(_), _) => self.text_run(&mut text)?,
                }
            }
        }
    }

    /// `<!--` or `<![CDATA[` inside an element.
    fn markup_in_content(&mut self, text: &mut TextBuf<'a>, kids: &mut Vec<Node>) -> Result<(), XmlError> {
        if self.starts(b"<!--") {
            self.flush(text, kids)?;
            let c = self.comment()?;
            kids.push(c);
            Ok(())
        } else if self.starts(b"<![CDATA[") {
            self.i += 9;
            let body = self.until(b"]]>", "an unterminated CDATA section")?;
            self.append_text(text, body)
        } else {
            Err(self.err("markup not allowed in content"))
        }
    }

    fn end_tag(&mut self, open: &str) -> Result<(), XmlError> {
        self.i += 2;
        let name = self.name("an end tag's name was expected")?;
        if name != open {
            return Err(self.err("an end tag does not match its start tag"));
        }
        self.skip_space();
        self.expect(b'>', "an end tag was not closed")
    }

    fn restore_scope(&mut self, len: usize, id: usize) {
        self.scope.truncate(len);
        self.scope_id = id;
    }

    /// A start tag at nesting `depth` (0 for the root): name, declarations, attributes.
    fn start_tag(&mut self, depth: usize) -> Result<StartTag<'a>, XmlError> {
        if depth >= self.limits.max_depth {
            return Err(XmlError::Limit {
                what: "element depth",
                limit: self.limits.max_depth,
            });
        }
        self.count_node()?;
        let tag_start = self.i;
        let tag = tag_end(self.s, tag_start).and_then(|end| self.s.get(tag_start..=end));
        if let Some(found) = tag.and_then(|bytes| self.interner.tag(self.scope_id, bytes)) {
            // a tag read before in this scope: the same head (it declares no namespaces)
            let raw = self.slice(tag_start + 1, tag_start + 1 + found.head.name.qualified().len());
            self.i = tag_start + tag.map_or(0, <[u8]>::len);
            let (scope_len, scope_id) = (self.scope.len(), self.scope_id);
            return Ok(StartTag {
                head: found.head,
                raw,
                empty: found.empty,
                scope_len,
                scope_id,
            });
        }
        self.i += 1;
        let at = self.i;
        let raw = self.name("an element name was expected")?;
        if !is_qname(raw) {
            return Err(XmlError::Syntax {
                offset: at,
                message: "an element name is not a valid QName",
            });
        }
        let (scope_len, scope_id) = (self.scope.len(), self.scope_id);
        // the buffers are reused from tag to tag (taken here, given back below)
        let mut namespaces = std::mem::take(&mut self.tag_namespaces);
        let mut raw_attributes = std::mem::take(&mut self.tag_attributes);
        let mut attributes = std::mem::take(&mut self.tag_resolved);
        namespaces.clear();
        raw_attributes.clear();
        attributes.clear();
        let empty = self.tag_body(&mut namespaces, &mut raw_attributes);
        let head = empty.and_then(|empty| {
            self.enter_scope(&namespaces);
            let name = self.resolve(raw, false, at)?;
            self.attributes(&raw_attributes, &mut attributes)?;
            Ok((self.interner.head(name, &namespaces, &attributes), empty))
        });
        self.tag_namespaces = namespaces;
        self.tag_attributes = raw_attributes;
        self.tag_resolved = attributes;
        let (head, empty) = head?;
        if head.namespaces.is_empty() {
            if let Some(bytes) = self.s.get(tag_start..self.i) {
                self.interner.remember_tag(scope_id, bytes, &head, empty);
            }
        }
        Ok(StartTag {
            head,
            raw,
            empty,
            scope_len,
            scope_id,
        })
    }

    /// The declarations and attributes of a start tag, up to `>` or `/>`; whether it was
    /// `/>`.
    fn tag_body(
        &mut self,
        namespaces: &mut Vec<NamespaceDecl>,
        raw_attributes: &mut Vec<(&'a str, usize, Atom)>,
    ) -> Result<bool, XmlError> {
        loop {
            let spaced = self.skip_space();
            match self.peek() {
                Some(b'>') => {
                    self.i += 1;
                    return Ok(false);
                }
                Some(b'/') => {
                    self.i += 1;
                    self.expect(b'>', "a start tag was not closed")?;
                    return Ok(true);
                }
                _ if !spaced => return Err(self.err("attributes must be separated by whitespace")),
                _ => {}
            }
            let offset = self.i;
            let name = self.name("an attribute name was expected")?;
            self.skip_space();
            self.expect(b'=', "an attribute without a value")?;
            self.skip_space();
            let value = self.attribute_value()?;
            if namespaces.len() + raw_attributes.len() >= self.limits.max_attributes {
                return Err(XmlError::Limit {
                    what: "attributes on an element",
                    limit: self.limits.max_attributes,
                });
            }
            if let Some(decl) = self.namespace_decl(name, offset, value.clone())? {
                namespaces.push(decl);
            } else if name != "xmlns:xml" {
                raw_attributes.push((name, offset, value));
            }
        }
    }

    /// `xmlns` / `xmlns:p` as a declaration; `None` for an ordinary attribute and for
    /// `xmlns:xml` bound to its own URI (which libxml2 drops).
    fn namespace_decl(&mut self, name: &str, offset: usize, uri: Atom) -> Result<Option<NamespaceDecl>, XmlError> {
        let prefix = match name.strip_prefix("xmlns") {
            Some("") => None,
            Some(rest) => match rest.strip_prefix(':') {
                Some(p) => Some(p),
                None => return Ok(None),
            },
            None => return Ok(None),
        };
        match prefix {
            Some("xml") if &*uri == XML_NAMESPACE => return Ok(None),
            Some("xml" | "xmlns") => {
                return Err(XmlError::Syntax {
                    offset,
                    message: "a reserved prefix redeclared",
                })
            }
            Some(p) if p.contains(':') || p.is_empty() => {
                return Err(XmlError::Syntax {
                    offset,
                    message: "a namespace prefix is not a valid NCName",
                })
            }
            Some(_) if uri.is_empty() => {
                return Err(XmlError::Syntax {
                    offset,
                    message: "an empty namespace URI for a prefix",
                })
            }
            _ => {}
        }
        let prefix = prefix.map(|p| self.interner.atom(p));
        Ok(Some(NamespaceDecl { prefix, uri }))
    }

    fn enter_scope(&mut self, declared: &[NamespaceDecl]) {
        if declared.is_empty() {
            return;
        }
        self.scope.extend(declared.iter().map(|d| Binding {
            prefix: d.prefix.clone(),
            uri: d.uri.clone(),
        }));
        self.scopes_made += 1;
        self.scope_id = self.scopes_made;
    }

    /// The shared name for `raw` in the current scope.
    fn resolve(&mut self, raw: &str, attribute: bool, offset: usize) -> Result<Name, XmlError> {
        let scope = &self.scope;
        self.interner.name(self.scope_id, attribute, raw, |interner| {
            let prefix = raw.split_once(':').map(|(p, _)| p);
            let uri = match prefix {
                Some("xml") => Some(XML_NAMESPACE),
                Some(p) => match scope.iter().rev().find(|b| b.prefix.as_deref() == Some(p)) {
                    Some(b) => Some(&*b.uri),
                    None => {
                        return Err(XmlError::UndefinedPrefix {
                            prefix: p.to_owned(),
                            offset,
                        })
                    }
                },
                None if attribute => None,
                None => scope
                    .iter()
                    .rev()
                    .find(|b| b.prefix.is_none())
                    .map(|b| &*b.uri)
                    .filter(|u| !u.is_empty()),
            };
            Ok(uri.map(|u| interner.atom(u)))
        })
    }

    /// Attributes resolved, with libxml2's duplicate checks (as written, and by namespace).
    fn attributes(&mut self, raw: &[(&'a str, usize, Atom)], out: &mut Vec<Attribute>) -> Result<(), XmlError> {
        // a set once there are many, so crafted input cannot make the check quadratic
        let mut seen: Option<HashSet<(Option<Atom>, String)>> = (raw.len() > 16).then(HashSet::new);
        for &(name, offset, ref value) in raw {
            if !is_qname(name) {
                return Err(XmlError::Syntax {
                    offset,
                    message: "an attribute name is not a valid QName",
                });
            }
            let name = self.resolve(name, true, offset)?;
            let duplicate = match seen.as_mut() {
                Some(seen) => {
                    let by_namespace = name
                        .namespace()
                        .map(|ns| (Some(Atom::from(ns)), name.local().to_owned()));
                    let as_written = !seen.insert((None, name.qualified().to_owned()));
                    as_written || by_namespace.is_some_and(|k| !seen.insert(k))
                }
                None => out.iter().any(|a| {
                    a.name.qualified() == name.qualified()
                        || (a.name.namespace().is_some()
                            && a.name.namespace() == name.namespace()
                            && a.name.local() == name.local())
                }),
            };
            if duplicate {
                return Err(XmlError::Syntax {
                    offset,
                    message: "an attribute is redefined",
                });
            }
            out.push(Attribute {
                name,
                value: value.clone(),
            });
        }
        Ok(())
    }
}
