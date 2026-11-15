//! A small XML reader for the packages' parts: elements with their namespaces resolved, their
//! attributes and the character data directly in them, read into a tree of [`Node`]s; and the
//! same tree written back ([`Node::write`]) with every namespace it names declared on its
//! root, so that a subtree can be handed on and parsed by itself.
//!
//! What a part's XML holds and this reader keeps: elements, attributes, character data
//! (entities and character references decoded, CDATA as text).  Comments, processing
//! instructions and the document type are skipped.  Anything it cannot read is an error
//! (the caller refuses the document; it never guesses).

use std::collections::HashMap;

/// A namespace's index in a [`Names`] table (0: no namespace).
pub type Ns = u16;

/// The namespace URIs a tree names, by index.
#[derive(Default, Clone, Debug)]
pub struct Names {
    pub uris: Vec<String>,
    index: HashMap<String, Ns>,
}

impl Names {
    pub fn new() -> Names {
        let mut n = Names::default();
        n.intern("");
        n
    }
    pub fn intern(&mut self, uri: &str) -> Ns {
        if let Some(&k) = self.index.get(uri) {
            return k;
        }
        let k = self.uris.len() as Ns;
        self.uris.push(uri.to_string());
        self.index.insert(uri.to_string(), k);
        k
    }
    pub fn get(&self, uri: &str) -> Option<Ns> {
        self.index.get(uri).copied()
    }
}

/// One attribute: its namespace, local name and value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr {
    pub ns: Ns,
    pub name: String,
    pub value: String,
}

/// One element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub ns: Ns,
    pub name: String,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
    /// The character data directly in it (between its children too), in order.
    pub text: String,
    /// The byte span of the element in the source (start of `<` to the end of its end tag).
    pub span: (usize, usize),
}

impl Node {
    pub fn is(&self, ns: Ns, name: &str) -> bool {
        self.ns == ns && self.name == name
    }
    /// The value of the attribute `name` in namespace `ns`.
    pub fn attr(&self, ns: Ns, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.ns == ns && a.name == name)
            .map(|a| a.value.as_str())
    }
    /// The value of the first attribute named `name`, whatever its namespace (as
    /// `ooxml_common.xmlutil.attr` reads one).
    pub fn attr_any(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.name == name).map(|a| a.value.as_str())
    }
    pub fn child(&self, ns: Ns, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.ns == ns && c.name == name)
    }
    pub fn children_named<'a>(&'a self, ns: Ns, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.ns == ns && c.name == name)
    }
    /// Every element of the subtree, itself first, in document order.
    pub fn descendants(&self) -> Vec<&Node> {
        let mut out = Vec::new();
        let mut stack = vec![self];
        while let Some(n) = stack.pop() {
            out.push(n);
            for c in n.children.iter().rev() {
                stack.push(c);
            }
        }
        out
    }
    /// The subtree as XML text, every namespace it names declared on its root (prefixes
    /// `n1`, `n2`... in the order first named; `w` for `w_uri` where given).
    pub fn write(&self, names: &Names, w_uri: Option<Ns>) -> String {
        let mut used: Vec<Ns> = Vec::new();
        fn collect(n: &Node, used: &mut Vec<Ns>) {
            if n.ns != 0 && !used.contains(&n.ns) {
                used.push(n.ns);
            }
            for a in &n.attrs {
                if a.ns != 0 && !used.contains(&a.ns) {
                    used.push(a.ns);
                }
            }
            for c in &n.children {
                collect(c, used);
            }
        }
        collect(self, &mut used);
        let mut prefixes: HashMap<Ns, String> = HashMap::new();
        let mut k = 0;
        for ns in &used {
            if Some(*ns) == w_uri {
                prefixes.insert(*ns, "w".into());
            } else if names.uris[*ns as usize] == "http://www.w3.org/XML/1998/namespace" {
                prefixes.insert(*ns, "xml".into());
            } else {
                k += 1;
                prefixes.insert(*ns, format!("n{k}"));
            }
        }
        let mut out = String::new();
        self.write_into(&mut out, &prefixes, Some((names, &used)));
        out
    }
    fn write_into(&self, out: &mut String, p: &HashMap<Ns, String>, root: Option<(&Names, &Vec<Ns>)>) {
        out.push('<');
        if self.ns != 0 {
            out.push_str(&p[&self.ns]);
            out.push(':');
        }
        out.push_str(&self.name);
        if let Some((names, used)) = root {
            for ns in used {
                if p[ns] == "xml" {
                    continue;
                }
                out.push_str(" xmlns:");
                out.push_str(&p[ns]);
                out.push_str("=\"");
                escape(&names.uris[*ns as usize], out, true);
                out.push('"');
            }
        }
        for a in &self.attrs {
            out.push(' ');
            if a.ns != 0 {
                out.push_str(&p[&a.ns]);
                out.push(':');
            }
            out.push_str(&a.name);
            out.push_str("=\"");
            escape(&a.value, out, true);
            out.push('"');
        }
        if self.children.is_empty() && self.text.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        escape(&self.text, out, false);
        for c in &self.children {
            c.write_into(out, p, None);
        }
        out.push_str("</");
        if self.ns != 0 {
            out.push_str(&p[&self.ns]);
            out.push(':');
        }
        out.push_str(&self.name);
        out.push('>');
    }
}

fn escape(s: &str, out: &mut String, attr: bool) {
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' if attr => out.push_str("&quot;"),
            '\n' if attr => out.push_str("&#10;"),
            '\t' if attr => out.push_str("&#9;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(c),
        }
    }
}

/// Why a part could not be read.
#[derive(Debug)]
pub struct XmlError(pub String);

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "xml: {}", self.0)
    }
}

/// A raw element while it is open: its qualified name, raw attributes, the namespace
/// declarations it made (prefix -> previous binding), its children so far and text.
struct Open {
    qname: String,
    attrs: Vec<(String, String)>,
    saved: Vec<(String, Option<Ns>)>,
    children: Vec<Node>,
    text: String,
    start: usize,
    ns: Ns,
    name: String,
    resolved: Vec<Attr>,
}

/// Reads a part (UTF-8) into its root element, namespaces interned in `names`.
pub fn parse(data: &[u8], names: &mut Names) -> Result<Node, XmlError> {
    let s = std::str::from_utf8(data).map_err(|_| XmlError("not UTF-8".into()))?;
    let b = s.as_bytes();
    let mut i = 0usize;
    let mut bindings: HashMap<String, Ns> = HashMap::new();
    bindings.insert("xml".into(), names.intern("http://www.w3.org/XML/1998/namespace"));
    let mut stack: Vec<Open> = Vec::new();
    let mut root: Option<Node> = None;
    let n = b.len();
    while i < n {
        if b[i] != b'<' {
            let end = memchr(b'<', &b[i..]).map(|k| i + k).unwrap_or(n);
            if let Some(top) = stack.last_mut() {
                decode(&s[i..end], &mut top.text)?;
            }
            i = end;
            continue;
        }
        let rest = &s[i..];
        if rest.starts_with("<?") {
            i += rest
                .find("?>")
                .ok_or_else(|| XmlError("an open processing instruction".into()))?
                + 2;
        } else if rest.starts_with("<!--") {
            i += rest.find("-->").ok_or_else(|| XmlError("an open comment".into()))? + 3;
        } else if rest.starts_with("<![CDATA[") {
            let end = rest
                .find("]]>")
                .ok_or_else(|| XmlError("an open CDATA section".into()))?;
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&rest[9..end]);
            }
            i += end + 3;
        } else if rest.starts_with("<!") {
            // a document type: skipped to its end (no internal subset is read)
            if rest.contains('[') && rest.find('[') < rest.find('>') {
                return Err(XmlError("a document type with an internal subset".into()));
            }
            i += rest.find('>').ok_or_else(|| XmlError("an open declaration".into()))? + 1;
        } else if rest.starts_with("</") {
            let end = rest.find('>').ok_or_else(|| XmlError("an open end tag".into()))?;
            let qname = rest[2..end].trim();
            let open = stack
                .pop()
                .ok_or_else(|| XmlError("an end tag with no element".into()))?;
            if open.qname != qname {
                return Err(XmlError(format!("</{qname}> closes <{}>", open.qname)));
            }
            i += end + 1;
            let node = close(open, i, &mut bindings);
            match stack.last_mut() {
                Some(parent) => parent.children.push(node),
                None => root = Some(node),
            }
        } else {
            // a start tag
            let start = i;
            let mut j = i + 1;
            while j < n && !matches!(b[j], b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/') {
                j += 1;
            }
            let qname = s[i + 1..j].to_string();
            let mut attrs: Vec<(String, String)> = Vec::new();
            let empty;
            loop {
                while j < n && matches!(b[j], b' ' | b'\t' | b'\r' | b'\n') {
                    j += 1;
                }
                if j >= n {
                    return Err(XmlError("an open start tag".into()));
                }
                if b[j] == b'>' {
                    empty = false;
                    j += 1;
                    break;
                }
                if b[j] == b'/' {
                    if j + 1 < n && b[j + 1] == b'>' {
                        empty = true;
                        j += 2;
                        break;
                    }
                    return Err(XmlError("a stray / in a start tag".into()));
                }
                let k0 = j;
                while j < n && !matches!(b[j], b'=' | b' ' | b'\t' | b'\r' | b'\n') {
                    j += 1;
                }
                let aname = s[k0..j].to_string();
                while j < n && matches!(b[j], b' ' | b'\t' | b'\r' | b'\n') {
                    j += 1;
                }
                if j >= n || b[j] != b'=' {
                    return Err(XmlError(format!("an attribute {aname} with no value")));
                }
                j += 1;
                while j < n && matches!(b[j], b' ' | b'\t' | b'\r' | b'\n') {
                    j += 1;
                }
                if j >= n || (b[j] != b'"' && b[j] != b'\'') {
                    return Err(XmlError("an unquoted attribute".into()));
                }
                let q = b[j];
                let v0 = j + 1;
                let v1 = memchr(q, &b[v0..])
                    .map(|k| v0 + k)
                    .ok_or_else(|| XmlError("an open attribute".into()))?;
                let mut value = String::new();
                decode_attr(&s[v0..v1], &mut value)?;
                attrs.push((aname, value));
                j = v1 + 1;
            }
            // namespace declarations first
            let mut saved = Vec::new();
            for (k, v) in &attrs {
                let prefix = if k == "xmlns" {
                    Some(String::new())
                } else {
                    k.strip_prefix("xmlns:").map(|p| p.to_string())
                };
                if let Some(p) = prefix {
                    let id = names.intern(v);
                    saved.push((p.clone(), bindings.insert(p, id)));
                }
            }
            let (ns, name) = resolve(&qname, &bindings, true)?;
            let mut resolved = Vec::new();
            for (k, v) in &attrs {
                if k == "xmlns" || k.starts_with("xmlns:") {
                    continue;
                }
                let (ans, aname) = resolve(k, &bindings, false)?;
                resolved.push(Attr {
                    ns: ans,
                    name: aname,
                    value: v.clone(),
                });
            }
            let open = Open {
                qname,
                attrs,
                saved,
                children: Vec::new(),
                text: String::new(),
                start,
                ns,
                name,
                resolved,
            };
            i = j;
            if empty {
                let node = close(open, i, &mut bindings);
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            } else {
                stack.push(open);
            }
        }
    }
    if !stack.is_empty() {
        return Err(XmlError("an element left open".into()));
    }
    root.ok_or_else(|| XmlError("no root element".into()))
}

fn close(open: Open, end: usize, bindings: &mut HashMap<String, Ns>) -> Node {
    for (p, before) in open.saved.into_iter().rev() {
        match before {
            Some(v) => {
                bindings.insert(p, v);
            }
            None => {
                bindings.remove(&p);
            }
        }
    }
    let _ = open.attrs;
    Node {
        ns: open.ns,
        name: open.name,
        attrs: open.resolved,
        children: open.children,
        text: open.text,
        span: (open.start, end),
    }
}

fn resolve(qname: &str, bindings: &HashMap<String, Ns>, element: bool) -> Result<(Ns, String), XmlError> {
    match qname.split_once(':') {
        Some((p, local)) => {
            let ns = *bindings
                .get(p)
                .ok_or_else(|| XmlError(format!("an undeclared prefix {p}")))?;
            Ok((ns, local.to_string()))
        }
        None => {
            let ns = if element {
                bindings.get("").copied().unwrap_or(0)
            } else {
                0
            };
            Ok((ns, qname.to_string()))
        }
    }
}

fn memchr(c: u8, b: &[u8]) -> Option<usize> {
    b.iter().position(|&x| x == c)
}

fn decode(raw: &str, out: &mut String) -> Result<(), XmlError> {
    if !raw.contains('&') && !raw.contains('\r') {
        out.push_str(raw);
        return Ok(());
    }
    let mut rest = raw;
    while let Some(k) = rest.find(['&', '\r']) {
        out.push_str(&rest[..k]);
        if rest.as_bytes()[k] == b'\r' {
            // line ends normalised as an XML processor does
            out.push('\n');
            rest = &rest[k + 1..];
            if rest.starts_with('\n') {
                rest = &rest[1..];
            }
            continue;
        }
        let end = rest[k..]
            .find(';')
            .ok_or_else(|| XmlError("an open reference".into()))?
            + k;
        let name = &rest[k + 1..end];
        match name {
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "amp" => out.push('&'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let code = if let Some(h) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                    u32::from_str_radix(h, 16).ok()
                } else if let Some(d) = name.strip_prefix('#') {
                    d.parse::<u32>().ok()
                } else {
                    None
                };
                let c = code
                    .and_then(char::from_u32)
                    .ok_or_else(|| XmlError(format!("an entity &{name};")))?;
                out.push(c);
            }
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(())
}

/// An attribute value: references decoded, and white space (tab, line ends) normalised to
/// spaces as an XML processor does (character references kept as written).
fn decode_attr(raw: &str, out: &mut String) -> Result<(), XmlError> {
    if raw.contains(['\t', '\n', '\r']) {
        let normal: String = raw
            .replace("\r\n", " ")
            .chars()
            .map(|c| if matches!(c, '\t' | '\n' | '\r') { ' ' } else { c })
            .collect();
        return decode(&normal, out);
    }
    decode(raw, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes() {
        let mut names = Names::new();
        let src = br#"<?xml version="1.0"?><w:p xmlns:w="urn:w" xmlns:x="urn:x"><w:r x:a="1 &amp; 2"><w:t xml:space="preserve"> a&lt;b&#x41;</w:t><!-- c --><w:tab/></w:r></w:p>"#;
        let root = parse(src, &mut names).unwrap();
        let w = names.get("urn:w").unwrap();
        assert!(root.is(w, "p"));
        let r = root.child(w, "r").unwrap();
        assert_eq!(r.attr(names.get("urn:x").unwrap(), "a"), Some("1 & 2"));
        assert_eq!(r.child(w, "t").unwrap().text, " a<bA");
        let again = r.write(&names, Some(w));
        let mut n2 = Names::new();
        let back = parse(again.as_bytes(), &mut n2).unwrap();
        assert_eq!(back.child(n2.get("urn:w").unwrap(), "t").unwrap().text, " a<bA");
    }
}
