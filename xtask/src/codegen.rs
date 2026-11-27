//! `cargo xtask codegen`: typed element views generated from a schema description.
//!
//! Owns: the description's grammar and the Rust it generates (design section 2.10, ADR-005).
//! A description (`crates/<crate>/schema/*.views`) lists, per element, the children and
//! attributes a reader asks for; the generator writes one view type per element with typed
//! getters (children by local name, or in exactly their namespace for a namespace-exact
//! target; attributes as strings, integers, on/off values). The output is checked in,
//! marked `@generated`; `--check` fails when it differs from what the description gives.
//!
//! Must not: hold hand-written logic for any element; the generated getters call the crate's
//! own reading helpers (`xmlread`), which define what "by local name" and an integer mean.
//!
//! The grammar, one item per line (`#` starts a comment):
//! - `ns <prefix> <uri>`: a namespace an exact-namespace attribute names;
//! - `view <Type> <local or *> | <doc>`: a view of the element with that local name (`*`:
//!   any element, read by what it holds);
//! - inside a view, indented: `child <fn> <View> <local>` (the first such child),
//!   `children <fn> <View> <local>` (all of them, in order), `all <fn> <View>` (every child
//!   element, in order), `attr <fn> <kind> <local>` with kind `str`, `num` (a finite number,
//!   as Python's `float()` reads it), `int`, `twips` (an exact target's: an integer or a
//!   universal measure such as strict's `35.40pt`, in twips), `onoff` (absent is on) or
//!   `flag` (`Some(value is not 0, false, off)`), `nsattr <fn> <prefix> <local>`, `text <fn>` (the element's text
//!   before its first child) and `alltext <fn>` (every text node below it);
//! - a view may name its namespace, `view <Type> <local> <prefix> | <doc>`, which gives it a
//!   constructor of the empty element (`create`); and inside a view, `order = <NAME>` names
//!   the schema's child sequence generated from the XML schemas (`ooxml_schema::orders::local`,
//!   [`crate::orders`]), which gives it `insert`: the element with a child added where the
//!   sequence puts it. `order <step> <step>...` (a step lists a choice's names as `a/b`) gives
//!   the sequence by hand, only for a namespace the vendored schemas lack (markup
//!   compatibility, package relationships).
//!
//! Each target names the module its getters call (`reader`): a crate's own (`crate::xmlread`,
//! the Word model's reading rules) or `ooxml_xml::view`.
//!
//! A namespace-exact target (`exact`, the Word model's: phase 3 identity design §3) matches
//! every name in its namespace, through its reader's `Ns` (one per vocabulary, a strict
//! document's URI held beside the transitional one):
//! - `ns <prefix> <Ns constant> [qualified]` declares a vocabulary (`ns w W qualified`:
//!   `xmlread::Ns::W`), `qualified` where its elements' attributes are in it (`w:val`) rather
//!   than in no namespace (DrawingML's `cx`);
//! - `view <Type> <local> <prefix> | <doc>`: the element `<prefix>:<local>`; a `*` view may
//!   name a prefix too, its children's and attributes' namespace;
//! - a child's namespace is the one its local names (`a:off`), else its view's (a view
//!   with a name), else its parent view's;
//! - an attribute is in its view's namespace where that is `qualified`, else in none.

use std::fmt::Write as _;
use std::path::Path;

/// One generated file: its description, its output, and the reading module it calls.
pub struct Target {
    /// The description, relative to the workspace root.
    pub description: &'static str,
    /// The generated module, relative to the workspace root.
    pub output: &'static str,
    /// The `use` line that brings the reading module in as `xmlread`.
    pub reader: &'static str,
    /// Whether names are matched in their namespaces (the reader's `Ns`), not by local name.
    pub exact: bool,
}

const SHARED: &str = "use ooxml_xml::view as xmlread;";
/// The crate that holds the generated orders (`crate::orders::local`), where `order =` works.
const GENERATED_ORDERS_CRATE: &str = "crates/ooxml-schema/";

/// Every description in the workspace.
pub const TARGETS: &[Target] = &[
    Target {
        description: "crates/ooxml-word-model/schema/w.views",
        output: "crates/ooxml-word-model/src/w.rs",
        reader: "use crate::xmlread;",
        exact: true,
    },
    Target {
        description: "crates/ooxml-schema/schema/rels.views",
        output: "crates/ooxml-schema/src/rels.rs",
        reader: SHARED,
        exact: false,
    },
    Target {
        description: "crates/ooxml-schema/schema/mc.views",
        output: "crates/ooxml-schema/src/mc.rs",
        reader: SHARED,
        exact: false,
    },
    Target {
        description: "crates/ooxml-schema/schema/a.views",
        output: "crates/ooxml-schema/src/a.rs",
        reader: SHARED,
        exact: false,
    },
    Target {
        description: "crates/ooxml-schema/schema/c.views",
        output: "crates/ooxml-schema/src/c.rs",
        reader: SHARED,
        exact: false,
    },
];

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
    All {
        name: String,
        view: String,
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
    AllText {
        name: String,
    },
}

#[derive(Debug)]
struct View {
    name: String,
    local: String,
    prefix: Option<String>,
    doc: String,
    items: Vec<Item>,
    order: Order,
}

/// A view's child sequence.
#[derive(Debug)]
enum Order {
    /// None: no `insert`.
    None,
    /// Given by hand (a namespace the vendored schemas lack).
    Hand(Vec<Vec<String>>),
    /// Generated: the name of an `ooxml_schema::orders::local` order.
    Generated(String),
}

/// A declared namespace: its prefix, its URI (or, in an exact target, the reader's `Ns`
/// constant), and whether its elements' attributes are in it.
struct Namespace {
    prefix: String,
    value: String,
    qualified: bool,
}

struct Description {
    namespaces: Vec<Namespace>,
    views: Vec<View>,
}

impl Description {
    fn namespace(&self, prefix: &str) -> Option<&Namespace> {
        self.namespaces.iter().find(|n| n.prefix == prefix)
    }

    fn view(&self, name: &str) -> Option<&View> {
        self.views.iter().find(|v| v.name == name)
    }

    /// In an exact target: the prefix of the namespace `local` (a child item's) is found in
    /// below `parent`, and its local name alone.
    fn child_namespace<'a>(&'a self, parent: &'a View, view: &str, local: &'a str) -> (Option<&'a str>, &'a str) {
        if let Some((prefix, name)) = local.split_once(':') {
            return (Some(prefix), name);
        }
        let own = self
            .view(view)
            .filter(|v| v.local != "*")
            .and_then(|v| v.prefix.as_deref());
        (own.or(parent.prefix.as_deref()), local)
    }
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
        Some("all") => Ok(Item::All {
            name: s(1)?,
            view: s(2)?,
        }),
        Some("attr") => {
            let kind = s(2)?;
            if !["str", "num", "int", "twips", "onoff", "flag"].contains(&kind.as_str()) {
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
        Some("alltext") => Ok(Item::AllText { name: s(1)? }),
        other => Err(format!("line {line_no}: unknown item {other:?}")),
    }
}

fn parse(source: &str, exact: bool) -> Result<Description, String> {
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
                Some("ns") => out.namespaces.push(Namespace {
                    prefix: fields.get(1).ok_or("ns: prefix")?.to_string(),
                    value: fields.get(2).ok_or("ns: uri")?.to_string(),
                    qualified: match fields.get(3).copied() {
                        None => false,
                        Some("qualified") => true,
                        Some(other) => return Err(format!("line {}: ns: unknown {other}", k + 1)),
                    },
                }),
                Some("view") => {
                    let (head, doc) = line
                        .split_once('|')
                        .ok_or(format!("line {}: a view needs `| doc`", k + 1))?;
                    let head = words(head);
                    out.views.push(View {
                        name: head.get(1).ok_or("view: name")?.to_string(),
                        local: head.get(2).ok_or("view: local name")?.to_string(),
                        prefix: head.get(3).map(|p| (*p).to_string()),
                        doc: doc.trim().to_string(),
                        items: Vec::new(),
                        order: Order::None,
                    });
                }
                other => return Err(format!("line {}: unknown statement {other:?}", k + 1)),
            }
            continue;
        }
        let view = out
            .views
            .last_mut()
            .ok_or(format!("line {}: an item outside a view", k + 1))?;
        if fields.first() == Some(&"order") {
            view.order = match fields.get(1..) {
                Some(["=", name]) => Order::Generated((*name).to_string()),
                Some(steps) if !steps.is_empty() && !steps.contains(&"=") => Order::Hand(
                    steps
                        .iter()
                        .map(|step| step.split('/').map(String::from).collect())
                        .collect(),
                ),
                _ => return Err(format!("line {}: order = <NAME>, or order <step>...", k + 1)),
            };
        } else {
            view.items.push(parse_item(&fields, k + 1)?);
        }
    }
    check(&out, exact)?;
    Ok(out)
}

fn check(description: &Description, exact: bool) -> Result<(), String> {
    let known: Vec<&str> = description.views.iter().map(|v| v.name.as_str()).collect();
    for view in &description.views {
        for item in &view.items {
            if let Item::Child { view: target, .. }
            | Item::Children { view: target, .. }
            | Item::All { view: target, .. } = item
            {
                if !known.contains(&target.as_str()) {
                    return Err(format!("{}: unknown view {target}", view.name));
                }
            }
            if let (false, Item::Attr { kind, .. }) = (exact, item) {
                if kind == "twips" {
                    return Err(format!("{}: twips needs a namespace-exact target", view.name));
                }
            }
            if let Item::NsAttr { prefix, .. } = item {
                if description.namespace(prefix).is_none() {
                    return Err(format!("{}: unknown namespace prefix {prefix}", view.name));
                }
            }
            if let (
                true,
                Item::Child {
                    view: target, local, ..
                }
                | Item::Children {
                    view: target, local, ..
                },
            ) = (exact, item)
            {
                match description.child_namespace(view, target, local).0 {
                    Some(prefix) if description.namespace(prefix).is_some() => {}
                    _ => return Err(format!("{}: {local}: no declared namespace for the child", view.name)),
                }
            }
        }
        match (&view.prefix, exact) {
            (Some(prefix), _) if description.namespace(prefix).is_none() => {
                return Err(format!("{}: unknown namespace prefix {prefix}", view.name));
            }
            (Some(_), false) if view.local == "*" => {
                return Err(format!("{}: a constructor needs a name", view.name));
            }
            (None, true) if view.local != "*" => {
                return Err(format!("{}: a view of a name needs its namespace", view.name));
            }
            _ => {}
        }
    }
    Ok(())
}

fn constant(prefix: &str) -> String {
    format!("NS_{}", prefix.to_uppercase())
}

/// How an exact target's getters name a namespace: the `NS_` constant of `prefix`.
fn ns_arg(prefix: &str) -> String {
    constant(prefix)
}

/// An exact target's getter of `item` in `view`, written; `false` for an item read the same
/// in every target.
fn emit_exact_item(out: &mut String, item: &Item, view: &View, description: &Description) -> bool {
    let _ = match item {
        Item::Child {
            name,
            view: target,
            local,
        } => {
            let (prefix, bare) = description.child_namespace(view, target, local);
            let ns = ns_arg(prefix.unwrap_or_default());
            let shown = format!("{}:{bare}", prefix.unwrap_or_default());
            writeln!(
                out,
                "    /// The first `{shown}` child.\n    pub fn {name}(self) -> Option<{target}<'a>> {{\n        \
                 xmlread::child_in(self.0, {ns}, \"{bare}\").map({target}::new)\n    }}"
            )
        }
        Item::Children {
            name,
            view: target,
            local,
        } => {
            let (prefix, bare) = description.child_namespace(view, target, local);
            let ns = ns_arg(prefix.unwrap_or_default());
            let shown = format!("{}:{bare}", prefix.unwrap_or_default());
            writeln!(
                out,
                "    /// Every `{shown}` child, in order.\n    pub fn {name}(self) -> impl Iterator<Item = {target}<'a>> {{\n        \
                 xmlread::children_in(self.0, {ns}, \"{bare}\").map({target}::new)\n    }}"
            )
        }
        Item::Attr { name, kind, local } => {
            let qualified = view
                .prefix
                .as_deref()
                .filter(|p| description.namespace(p).is_some_and(|n| n.qualified));
            let (ns, shown) = match qualified {
                Some(prefix) => (format!("Some({})", ns_arg(prefix)), format!("{prefix}:{local}")),
                None => ("None".to_owned(), local.clone()),
            };
            let (ty, call) = match kind.as_str() {
                "str" => ("Option<&'a str>", "xmlread::attr_in"),
                "int" => ("Option<i64>", "xmlread::attr_int_in"),
                "twips" => ("Option<i64>", "xmlread::attr_twips_in"),
                "onoff" => ("bool", "xmlread::attr_on_in"),
                _ => ("Option<bool>", "xmlread::attr_flag_in"),
            };
            writeln!(
                out,
                "    /// `@{shown}` ({kind}).\n    pub fn {name}(self) -> {ty} {{\n        {call}(self.0, {ns}, \"{local}\")\n    }}"
            )
        }
        Item::NsAttr { name, prefix, local } => writeln!(
            out,
            "    /// `@{prefix}:{local}`.\n    pub fn {name}(self) -> Option<&'a str> {{\n        \
             xmlread::attr_in(self.0, Some({}), \"{local}\")\n    }}",
            ns_arg(prefix)
        ),
        _ => return false,
    };
    true
}

fn emit_item(out: &mut String, item: &Item, view: &View, description: &Description, exact: bool) {
    if exact && emit_exact_item(out, item, view, description) {
        return;
    }
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
        Item::All { name, view } => writeln!(
            out,
            "    /// Every child element, in order.\n    pub fn {name}(self) -> impl Iterator<Item = {view}<'a>> {{\n        \
             xmlread::elements(self.0).map({view}::new)\n    }}"
        ),
        Item::Attr { name, kind, local } => {
            let (ty, call) = match kind.as_str() {
                "str" => ("Option<&'a str>", "xmlread::attr"),
                "num" => ("Option<f64>", "xmlread::attr_num"),
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
        Item::AllText { name } => writeln!(
            out,
            "    /// Every text node below the element, in order.\n    pub fn {name}(self) -> String {{\n        \
             self.0.text_content()\n    }}"
        ),
    };
}

fn emit_view(out: &mut String, view: &View, description: &Description, exact: bool) {
    let name = &view.name;
    let _ = writeln!(
        out,
        "\n/// {}\n#[derive(Debug, Clone, Copy)]\npub struct {name}<'a>(&'a Element);",
        view.doc
    );
    let _ = writeln!(out, "\nimpl<'a> {name}<'a> {{");
    if let (true, "*") = (exact, view.local.as_str()) {
    } else if exact {
        let local = &view.local;
        let prefix = view.prefix.as_deref().unwrap_or_default();
        let ns = ns_arg(prefix);
        let _ = writeln!(
            out,
            "    /// The local name this view reads.\n    pub const LOCAL: &'static str = \"{local}\";\n\n    \
             /// `element` as this view, if it is `{prefix}:{local}`.\n    pub fn of(element: &'a Element) -> Option<Self> {{\n        \
             {ns}.named(element, Self::LOCAL).then_some(Self(element))\n    }}\n"
        );
    } else if view.local != "*" {
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
        emit_item(out, item, view, description, exact);
    }
    emit_constructors(out, view, exact);
    out.push_str("}\n");
}

/// A view in a known namespace: its qualified name and the empty element; with an `order`
/// (in any namespace), the schema's child sequence and `insert`.
fn emit_constructors(out: &mut String, view: &View, exact: bool) {
    if let (Some(prefix), false) = (&view.prefix, exact) {
        emit_create(out, &view.local, prefix);
    }
    emit_order(out, view);
}

fn emit_create(out: &mut String, local: &str, prefix: &str) {
    let ns = constant(prefix);
    let _ = writeln!(
        out,
        "\n    /// The element this view reads, as written: `{prefix}:{local}`.\n    \
         pub const QUALIFIED: &'static str = \"{prefix}:{local}\";\n\n    \
         /// A new, empty `{prefix}:{local}`.\n    pub fn create() -> Element {{\n        \
         Element::new(Arc::new(QName::new(Some(\"{prefix}\"), \"{local}\", Some({ns}))), Vec::new(), Vec::new(), Vec::new())\n    }}"
    );
}

fn emit_order(out: &mut String, view: &View) {
    let value = match &view.order {
        Order::None => return,
        Order::Generated(name) => format!("crate::orders::local::{name}"),
        Order::Hand(order) => {
            let steps: Vec<String> = order
                .iter()
                .map(|step| {
                    format!(
                        "&[{}]",
                        step.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ")
                    )
                })
                .collect();
            format!("&[{}]", steps.join(", "))
        }
    };
    let _ = writeln!(
        out,
        "\n    /// The schema's sequence of children, by local name (a step lists a choice).\n    \
         pub const ORDER: &'static [&'static [&'static str]] = {value};\n\n    \
         /// The element with `child` added where [`Self::ORDER`] puts it.\n    \
         pub fn insert(self, child: Element) -> Element {{\n        \
         xmlread::insert_ordered(self.0, Self::ORDER, child)\n    }}",
    );
}

fn generate(description: &Description, target: &Target) -> String {
    let (source_path, reader) = (target.description, target.reader);
    let found = if target.exact {
        "in exactly their namespaces (`xmlread::Ns`: a\n//! strict document's names too)"
    } else {
        "by local name as `xmlread` reads them"
    };
    let mut out = format!(
        "// @generated by `cargo xtask codegen` from {source_path}; do not edit by hand.\n\
         //! Typed views of the elements the model reads (design ADR-005), generated from\n\
         //! `{source_path}`: each a copy of a borrowed element with getters for the children\n\
         //! and attributes asked for, found {found}.\n\
         //!\n//! Must not: hold logic; a reader that needs more adds it to the description.\n\
         #![allow(clippy::all)]\n\n{reader}\nuse ooxml_xml::Element;\n"
    );
    if !target.exact && description.views.iter().any(|v| v.prefix.is_some()) {
        out.push_str("use ooxml_xml::QName;\nuse std::sync::Arc;\n");
    }
    for ns in &description.namespaces {
        let (prefix, value) = (&ns.prefix, &ns.value);
        let _ = if target.exact {
            writeln!(
                out,
                "\n/// The `{prefix}:` namespace.\npub const {}: xmlread::Ns = xmlread::Ns::{value};",
                constant(prefix)
            )
        } else {
            writeln!(
                out,
                "\n/// The `{prefix}:` namespace.\npub const {}: &str = \"{value}\";",
                constant(prefix)
            )
        };
    }
    for view in &description.views {
        emit_view(&mut out, view, description, target.exact);
    }
    out
}

/// Regenerate every target under `root`; with `check`, report the targets that differ
/// instead of writing them.
pub fn run(root: &Path, check: bool) -> Result<Vec<String>, String> {
    let mut differ = Vec::new();
    let locals = crate::orders::local_names(root)?;
    for target in TARGETS {
        let source = std::fs::read_to_string(root.join(target.description)).map_err(|e| e.to_string())?;
        let description = parse(&source, target.exact)?;
        for view in &description.views {
            if let Order::Generated(name) = &view.order {
                if !target.output.starts_with(GENERATED_ORDERS_CRATE) || !locals.contains(name) {
                    return Err(format!(
                        "{}: {}: order = {name} names no local order in {GENERATED_ORDERS_CRATE}",
                        target.description, view.name
                    ));
                }
            }
        }
        let generated = generate(&description, target);
        let path = root.join(target.output);
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current != generated {
            differ.push(target.output.to_string());
            if !check {
                std::fs::write(&path, generated).map_err(|e| e.to_string())?;
            }
        }
    }
    differ.extend(crate::orders::run(root, check)?);
    Ok(differ)
}
