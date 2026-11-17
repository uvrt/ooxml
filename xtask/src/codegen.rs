//! `cargo xtask codegen`: typed element views generated from a schema description.
//!
//! Owns: the description's grammar and the Rust it generates (design section 2.10, ADR-005).
//! A description (`crates/<crate>/schema/*.views`) lists, per element, the children and
//! attributes a reader asks for; the generator writes one view type per element with typed
//! getters (children by local name, attributes as strings, integers, on/off values). The
//! output is checked in, marked `@generated`; `--check` fails when it differs from what the
//! description gives.
//!
//! Must not: hold hand-written logic for any element; the generated getters call the crate's
//! own reading helpers (`xmlread`), which define what "by local name" and an integer mean.
//!
//! The grammar, one item per line (`#` starts a comment):
//! - `ns <prefix> <uri>`: a namespace an exact-namespace attribute names;
//! - `view <Type> <local or *> | <doc>`: a view of the element with that local name (`*`:
//!   any element, read by what it holds);
//! - inside a view, indented: `child <fn> <View> <local>` (the first such child),
//!   `children <fn> <View> <local>` (all of them, in order), `attr <fn> <kind> <local>`
//!   with kind `str`, `int`, `onoff` (absent is on) or `flag` (`Some(value is not 0, false,
//!   off)`), `nsattr <fn> <prefix> <local>`, and `text <fn>` (the element's text).

use std::fmt::Write as _;
use std::path::Path;

/// One generated file: its description and its output.
pub struct Target {
    /// The description, relative to the workspace root.
    pub description: &'static str,
    /// The generated module, relative to the workspace root.
    pub output: &'static str,
}

/// Every description in the workspace.
pub const TARGETS: &[Target] = &[Target {
    description: "crates/ooxml-word-model/schema/w.views",
    output: "crates/ooxml-word-model/src/w.rs",
}];

#[derive(Debug)]
enum Item {
    Child {
        name: String,
        view: String,
        local: String,
    },
    Children {
        name: String,
        view: String,
        local: String,
    },
    Attr {
        name: String,
        kind: String,
        local: String,
    },
    NsAttr {
        name: String,
        prefix: String,
        local: String,
    },
    Text {
        name: String,
    },
}

#[derive(Debug)]
struct View {
    name: String,
    local: String,
    doc: String,
    items: Vec<Item>,
}

struct Description {
    namespaces: Vec<(String, String)>,
    views: Vec<View>,
}

fn words(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

fn parse_item(fields: &[&str], line_no: usize) -> Result<Item, String> {
    let s = |i: usize| {
        fields
            .get(i)
            .map(|w| (*w).to_string())
            .ok_or(format!("line {line_no}: too few fields"))
    };
    match fields.first().copied() {
        Some("child") => Ok(Item::Child {
            name: s(1)?,
            view: s(2)?,
            local: s(3)?,
        }),
        Some("children") => Ok(Item::Children {
            name: s(1)?,
            view: s(2)?,
            local: s(3)?,
        }),
        Some("attr") => {
            let kind = s(2)?;
            if !["str", "int", "onoff", "flag"].contains(&kind.as_str()) {
                return Err(format!("line {line_no}: unknown attribute kind {kind}"));
            }
            Ok(Item::Attr {
                name: s(1)?,
                kind,
                local: s(3)?,
            })
        }
        Some("nsattr") => Ok(Item::NsAttr {
            name: s(1)?,
            prefix: s(2)?,
            local: s(3)?,
        }),
        Some("text") => Ok(Item::Text { name: s(1)? }),
        other => Err(format!("line {line_no}: unknown item {other:?}")),
    }
}

fn parse(source: &str) -> Result<Description, String> {
    let mut out = Description {
        namespaces: Vec::new(),
        views: Vec::new(),
    };
    for (k, raw) in source.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("");
        if line.trim().is_empty() {
            continue;
        }
        let fields = words(line);
        if !line.starts_with(' ') {
            match fields.first().copied() {
                Some("ns") => out.namespaces.push((
                    fields.get(1).ok_or("ns: prefix")?.to_string(),
                    fields.get(2).ok_or("ns: uri")?.to_string(),
                )),
                Some("view") => {
                    let (head, doc) = line
                        .split_once('|')
                        .ok_or(format!("line {}: a view needs `| doc`", k + 1))?;
                    let head = words(head);
                    out.views.push(View {
                        name: head.get(1).ok_or("view: name")?.to_string(),
                        local: head.get(2).ok_or("view: local name")?.to_string(),
                        doc: doc.trim().to_string(),
                        items: Vec::new(),
                    });
                }
                other => return Err(format!("line {}: unknown statement {other:?}", k + 1)),
            }
            continue;
        }
        let item = parse_item(&fields, k + 1)?;
        out.views
            .last_mut()
            .ok_or(format!("line {}: an item outside a view", k + 1))?
            .items
            .push(item);
    }
    check(&out)?;
    Ok(out)
}

fn check(description: &Description) -> Result<(), String> {
    let known: Vec<&str> = description.views.iter().map(|v| v.name.as_str()).collect();
    for view in &description.views {
        for item in &view.items {
            if let Item::Child { view: target, .. } | Item::Children { view: target, .. } = item {
                if !known.contains(&target.as_str()) {
                    return Err(format!("{}: unknown view {target}", view.name));
                }
            }
            if let Item::NsAttr { prefix, .. } = item {
                if !description.namespaces.iter().any(|(p, _)| p == prefix) {
                    return Err(format!("{}: unknown namespace prefix {prefix}", view.name));
                }
            }
        }
    }
    Ok(())
}

fn constant(prefix: &str) -> String {
    format!("NS_{}", prefix.to_uppercase())
}

fn emit_item(out: &mut String, item: &Item) {
    let _ = match item {
        Item::Child { name, view, local } => writeln!(
            out,
            "    /// The first `{local}` child.\n    pub fn {name}(self) -> Option<{view}<'a>> {{\n        \
             xmlread::child(self.0, \"{local}\").map({view}::new)\n    }}"
        ),
        Item::Children { name, view, local } => writeln!(
            out,
            "    /// Every `{local}` child, in order.\n    pub fn {name}(self) -> impl Iterator<Item = {view}<'a>> {{\n        \
             xmlread::children(self.0, \"{local}\").map({view}::new)\n    }}"
        ),
        Item::Attr { name, kind, local } => {
            let (ty, call) = match kind.as_str() {
                "str" => ("Option<&'a str>", "xmlread::attr"),
                "int" => ("Option<i64>", "xmlread::attr_int"),
                "onoff" => ("bool", "xmlread::attr_on"),
                _ => ("Option<bool>", "xmlread::attr_flag"),
            };
            writeln!(
                out,
                "    /// `@{local}` ({kind}).\n    pub fn {name}(self) -> {ty} {{\n        {call}(self.0, \"{local}\")\n    }}"
            )
        }
        Item::NsAttr { name, prefix, local } => writeln!(
            out,
            "    /// `@{prefix}:{local}`.\n    pub fn {name}(self) -> Option<&'a str> {{\n        \
             xmlread::attr_ns(self.0, {}, \"{local}\")\n    }}",
            constant(prefix)
        ),
        Item::Text { name } => writeln!(
            out,
            "    /// The element's text (before its first child element).\n    pub fn {name}(self) -> String {{\n        \
             xmlread::text(self.0)\n    }}"
        ),
    };
}

fn emit_view(out: &mut String, view: &View) {
    let name = &view.name;
    let _ = writeln!(
        out,
        "\n/// {}\n#[derive(Debug, Clone, Copy)]\npub struct {name}<'a>(&'a Element);",
        view.doc
    );
    let _ = writeln!(out, "\nimpl<'a> {name}<'a> {{");
    if view.local != "*" {
        let local = &view.local;
        let _ = writeln!(
            out,
            "    /// The local name this view reads.\n    pub const LOCAL: &'static str = \"{local}\";\n\n    \
             /// `element` as this view, if its local name is `{local}`.\n    pub fn of(element: &'a Element) -> Option<Self> {{\n        \
             (element.name().local() == Self::LOCAL).then_some(Self(element))\n    }}\n"
        );
    }
    let _ = writeln!(
        out,
        "    /// `element` read as this view, whatever its name.\n    pub fn new(element: &'a Element) -> Self {{\n        \
         Self(element)\n    }}\n\n    /// The element.\n    pub fn element(self) -> &'a Element {{\n        self.0\n    }}"
    );
    for item in &view.items {
        out.push('\n');
        emit_item(out, item);
    }
    out.push_str("}\n");
}

fn generate(description: &Description, source_path: &str) -> String {
    let mut out = format!(
        "// @generated by `cargo xtask codegen` from {source_path}; do not edit by hand.\n\
         //! Typed views of the elements the model reads (design ADR-005), generated from\n\
         //! `{source_path}`: each a copy of a borrowed element with getters for the children\n\
         //! and attributes asked for, found by local name as `xmlread` reads them.\n\
         //!\n//! Must not: hold logic; a reader that needs more adds it to the description.\n\
         #![allow(clippy::all)]\n\nuse crate::xmlread;\nuse ooxml_xml::Element;\n"
    );
    for (prefix, uri) in &description.namespaces {
        let _ = writeln!(
            out,
            "\n/// The `{prefix}:` namespace.\npub const {}: &str = \"{uri}\";",
            constant(prefix)
        );
    }
    for view in &description.views {
        emit_view(&mut out, view);
    }
    out
}

/// Regenerate every target under `root`; with `check`, report the targets that differ
/// instead of writing them.
pub fn run(root: &Path, check: bool) -> Result<Vec<String>, String> {
    let mut differ = Vec::new();
    for target in TARGETS {
        let source = std::fs::read_to_string(root.join(target.description)).map_err(|e| e.to_string())?;
        let generated = generate(&parse(&source)?, target.description);
        let path = root.join(target.output);
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current != generated {
            differ.push(target.output.to_string());
            if !check {
                std::fs::write(&path, generated).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(differ)
}
