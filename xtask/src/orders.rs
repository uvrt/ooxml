//! `cargo xtask codegen` (with the views): the schemas' child orders, generated from the
//! published XML schemas.
//!
//! Owns: reading the XSDs a description names (`crates/ooxml-schema/schema/orders.list`), the
//! rule that turns a content model into ranks, and `crates/ooxml-schema/src/orders.rs`.
//!
//! A parent's ranks are the steps of its type's content model: an element is a step; a
//! sequence (or a group holding one) is its steps in turn; a choice of elements alone is one
//! step (a repeating choice says nothing about order); a choice holding a sequence lists its
//! alternatives one after another (they never meet in one parent, so their order is free);
//! an extension is its base's steps, then its own. A parent written by several types (`c:ser`
//! of every chart kind) takes the first type's steps and places each name another type adds
//! right after the step of the name before it there. The sources go in order: the first
//! gives the order, a later one places only the names the earlier ones lack, in the same way.
//! A parent with an overlay takes the overlay's order instead.
//!
//! Must not: hold an order by hand (that is the overlay's, with its cause), or read a schema
//! it does not name: a group, type or namespace a parent needs and the sources lack is an
//! error, never a silently shorter order.

use ooxml_xml::Element;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

const XS: &str = "http://www.w3.org/2001/XMLSchema";
const DESCRIPTION: &str = "crates/ooxml-schema/schema/orders.list";
const SCHEMAS: &str = "crates/ooxml-schema/xsd";
/// The generated module.
pub const OUTPUT: &str = "crates/ooxml-schema/src/orders.rs";

/// A name in a namespace: `(uri, local)`.
type Q = (String, String);
/// Steps, each a list of names.
type Ranks = Vec<Vec<Q>>;

/// What the description says.
#[derive(Default)]
struct Description {
    sources: Vec<String>,
    /// `(prefix, uri)`, in order.
    namespaces: Vec<(String, String)>,
    parents: Vec<String>,
    /// Parent to its overlay: its cause and its steps as written.
    overlays: BTreeMap<String, (String, Vec<Vec<String>>)>,
}

fn parse(source: &str) -> Result<Description, String> {
    let mut out = Description::default();
    let mut cause: Option<String> = None;
    for (k, line) in source.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = || format!("{DESCRIPTION}:{}", k + 1);
        let (word, rest) = line.split_once(' ').ok_or_else(|| format!("{}: no value", at()))?;
        match word {
            "source" => out.sources.push(rest.to_owned()),
            "ns" => {
                let (prefix, uri) = rest
                    .split_once(' ')
                    .ok_or_else(|| format!("{}: ns <prefix> <uri>", at()))?;
                out.namespaces.push((prefix.to_owned(), uri.to_owned()));
            }
            "parent" => out.parents.push(rest.to_owned()),
            "cause" => cause = Some(rest.to_owned()),
            "overlay" => {
                let (parent, steps) = rest
                    .split_once(" = ")
                    .ok_or_else(|| format!("{}: overlay <name> = ...", at()))?;
                let why = cause
                    .take()
                    .ok_or_else(|| format!("{}: an overlay needs its cause line", at()))?;
                if !(why.starts_with("frozen Python, unmeasured: ") || why.starts_with("measured in Word")) {
                    return Err(format!(
                        "{}: a cause is measured in Word or frozen Python, unmeasured",
                        at()
                    ));
                }
                let steps = steps
                    .split(" | ")
                    .map(|s| s.split(' ').filter(|n| !n.is_empty()).map(str::to_owned).collect())
                    .collect();
                out.overlays.insert(parent.to_owned(), (why, steps));
            }
            other => return Err(format!("{}: unknown item {other}", at())),
        }
    }
    Ok(out)
}

/// One schema node with the namespaces its file declares, to resolve the names it refers to.
#[derive(Clone)]
struct Node {
    element: Arc<Element>,
    file: Arc<FileNames>,
}

struct FileNames {
    target: String,
    /// Prefix (empty: the default namespace) to URI.
    prefixes: BTreeMap<String, String>,
}

impl FileNames {
    fn resolve(&self, name: &str) -> Result<Q, String> {
        match name.split_once(':') {
            Some((prefix, local)) => self
                .prefixes
                .get(prefix)
                .map(|uri| (uri.clone(), local.to_owned()))
                .ok_or_else(|| format!("no namespace for the prefix of {name}")),
            None => Ok((
                self.prefixes.get("").cloned().unwrap_or_else(|| self.target.clone()),
                name.to_owned(),
            )),
        }
    }
}

/// One source's schemas: its types, groups and which types each element name is written by.
#[derive(Default)]
struct Schemas {
    types: BTreeMap<Q, Node>,
    groups: BTreeMap<Q, Node>,
    elements: BTreeMap<Q, BTreeSet<Q>>,
}

fn is_xs(element: &Element, local: &str) -> bool {
    element.name().is(Some(XS), local)
}

fn load(dir: &Path) -> Result<Schemas, String> {
    let mut out = Schemas::default();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "xsd"))
        .collect();
    files.sort();
    for path in files {
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let tree =
            ooxml_xml::parse(&bytes, &ooxml_xml::Limits::default()).map_err(|e| format!("{}: {e}", path.display()))?;
        let root = tree.root();
        let target = root.attribute(None, "targetNamespace").unwrap_or("").to_owned();
        let prefixes = root
            .namespaces()
            .iter()
            .map(|d| (d.prefix.as_deref().unwrap_or("").to_owned(), d.uri.to_string()))
            .collect();
        let file = Arc::new(FileNames {
            target: target.clone(),
            prefixes,
        });
        for child in root.child_elements() {
            let Some(name) = child.attribute(None, "name") else {
                continue;
            };
            let key = (target.clone(), name.to_owned());
            let node = Node {
                element: Arc::clone(child),
                file: Arc::clone(&file),
            };
            if is_xs(child, "complexType") {
                out.types.entry(key).or_insert(node);
            } else if is_xs(child, "group") {
                out.groups.entry(key).or_insert(node);
            }
        }
        declarations(root, &file, &target, &mut out.elements)?;
    }
    Ok(out)
}

/// Every element declared with a name and a type, global or local, by name to its types.
fn declarations(
    element: &Element,
    file: &FileNames,
    target: &str,
    out: &mut BTreeMap<Q, BTreeSet<Q>>,
) -> Result<(), String> {
    for child in element.child_elements() {
        if is_xs(child, "element") {
            if let (Some(name), Some(kind)) = (child.attribute(None, "name"), child.attribute(None, "type")) {
                out.entry((target.to_owned(), name.to_owned()))
                    .or_default()
                    .insert(file.resolve(kind)?);
            }
        }
        declarations(child, file, target, out)?;
    }
    Ok(())
}

fn particles(element: &Element) -> impl Iterator<Item = &Arc<Element>> {
    element.child_elements().filter(|c| {
        ["element", "group", "sequence", "choice", "all", "any"]
            .iter()
            .any(|l| is_xs(c, l))
    })
}

impl Schemas {
    fn group(&self, node: &Node) -> Result<Node, String> {
        let name = node.element.attribute(None, "ref").ok_or("a group without ref")?;
        let key = node.file.resolve(name)?;
        self.groups
            .get(&key)
            .cloned()
            .ok_or_else(|| format!("no group {}:{}", key.0, key.1))
    }

    /// The element names below a particle, in order.
    fn names(&self, node: &Node, out: &mut Vec<Q>) -> Result<(), String> {
        let e = &node.element;
        if is_xs(e, "element") {
            match (e.attribute(None, "ref"), e.attribute(None, "name")) {
                (Some(r), _) => out.push(node.file.resolve(r)?),
                (None, Some(n)) => out.push((node.file.target.clone(), n.to_owned())),
                (None, None) => return Err("an element without name or ref".into()),
            }
        } else if is_xs(e, "group") && e.attribute(None, "ref").is_some() {
            let group = self.group(node)?;
            self.names(&group, out)?;
        } else if !is_xs(e, "any") {
            for c in particles(e) {
                self.names(
                    &Node {
                        element: Arc::clone(c),
                        file: Arc::clone(&node.file),
                    },
                    out,
                )?;
            }
        }
        Ok(())
    }

    /// Whether a particle is a choice of elements alone (one step).
    fn simple(&self, node: &Node) -> Result<bool, String> {
        let e = &node.element;
        if is_xs(e, "element") || is_xs(e, "any") {
            return Ok(true);
        }
        if is_xs(e, "group") && e.attribute(None, "ref").is_some() {
            let group = self.group(node)?;
            return self.simple(&group);
        }
        if is_xs(e, "choice") || is_xs(e, "group") {
            for c in particles(e) {
                if !self.simple(&Node {
                    element: Arc::clone(c),
                    file: Arc::clone(&node.file),
                })? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// The steps of a particle in a sequence.
    fn steps(&self, node: &Node, out: &mut Ranks) -> Result<(), String> {
        let e = &node.element;
        let child = |c: &Arc<Element>| Node {
            element: Arc::clone(c),
            file: Arc::clone(&node.file),
        };
        if is_xs(e, "sequence") {
            for c in particles(e) {
                self.steps(&child(c), out)?;
            }
        } else if is_xs(e, "group") && e.attribute(None, "ref").is_some() {
            let group = self.group(node)?;
            for c in particles(&group.element) {
                self.steps(
                    &Node {
                        element: Arc::clone(c),
                        file: Arc::clone(&group.file),
                    },
                    out,
                )?;
            }
        } else if (is_xs(e, "choice") || is_xs(e, "all")) && !self.simple(node)? {
            for c in particles(e) {
                self.steps(&child(c), out)?;
            }
        } else {
            let mut names = Vec::new();
            self.names(node, &mut names)?;
            if !names.is_empty() {
                out.push(names);
            }
        }
        Ok(())
    }

    /// The steps of a type's content.
    fn type_steps(&self, kind: &Q) -> Result<Option<Ranks>, String> {
        let Some(node) = self.types.get(kind) else {
            return Ok(None);
        };
        let mut out = Vec::new();
        self.body(node, &mut out)?;
        Ok(Some(out))
    }

    fn body(&self, node: &Node, out: &mut Ranks) -> Result<(), String> {
        for c in node.element.child_elements() {
            let here = Node {
                element: Arc::clone(c),
                file: Arc::clone(&node.file),
            };
            if ["sequence", "choice", "all", "group"].iter().any(|l| is_xs(c, l)) {
                self.steps(&here, out)?;
            } else if is_xs(c, "complexContent") {
                for x in c.child_elements().filter(|x| is_xs(x, "extension")) {
                    let base = x.attribute(None, "base").ok_or("an extension without base")?;
                    let key = node.file.resolve(base)?;
                    let base = self
                        .types
                        .get(&key)
                        .ok_or_else(|| format!("no type {}:{}", key.0, key.1))?;
                    self.body(base, out)?;
                    self.body(
                        &Node {
                            element: Arc::clone(x),
                            file: Arc::clone(&node.file),
                        },
                        out,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Each type writing `parent`, the largest first (by names, then by type name).
    fn parent_steps(&self, parent: &Q) -> Result<Vec<Ranks>, String> {
        let mut all = Vec::new();
        for kind in self.elements.get(parent).into_iter().flatten() {
            if let Some(steps) = self.type_steps(kind)? {
                all.push(steps);
            }
        }
        all.sort_by_key(|s| std::cmp::Reverse(s.iter().map(Vec::len).sum::<usize>()));
        Ok(all)
    }
}

/// `other`'s names `base` lacks, each right after the step of the name before it in `other`
/// (in that name's step when a name of its own step is known).
fn merge(mut base: Ranks, other: &Ranks) -> Ranks {
    let mut known: BTreeSet<Q> = base.iter().flatten().cloned().collect();
    let mut previous: Option<Q> = None;
    for step in other {
        let missing: Vec<Q> = step.iter().filter(|n| !known.contains(*n)).cloned().collect();
        if !missing.is_empty() {
            let present = step.iter().find(|n| known.contains(*n));
            match present.and_then(|p| base.iter().position(|s| s.contains(p))) {
                Some(j) => {
                    if let Some(s) = base.get_mut(j) {
                        s.extend(missing.iter().cloned());
                    }
                }
                None => {
                    let at = previous
                        .as_ref()
                        .and_then(|p| base.iter().position(|s| s.contains(p)))
                        .map_or(0, |i| i + 1);
                    base.insert(at, missing.clone());
                }
            }
            known.extend(missing);
        }
        if let Some(last) = step.last() {
            previous = Some(last.clone());
        }
    }
    base
}

/// The generated ranks of every listed parent, as written names.
fn generate_orders(root: &Path, d: &Description) -> Result<Vec<(String, Vec<Vec<String>>, Option<String>)>, String> {
    let sources: Vec<Schemas> = d
        .sources
        .iter()
        .map(|s| load(&root.join(SCHEMAS).join(s)))
        .collect::<Result<_, _>>()?;
    let uri_of = |prefix: &str| d.namespaces.iter().find(|(p, _)| p == prefix).map(|(_, u)| u.clone());
    let prefix_of = |uri: &str| d.namespaces.iter().find(|(_, u)| u == uri).map(|(p, _)| p.clone());
    let mut out = Vec::new();
    for parent in &d.parents {
        if let Some((cause, steps)) = d.overlays.get(parent) {
            out.push((parent.clone(), steps.clone(), Some(cause.clone())));
            continue;
        }
        let (prefix, local) = parent
            .split_once(':')
            .ok_or_else(|| format!("{parent}: not prefixed"))?;
        let key = (
            uri_of(prefix).ok_or_else(|| format!("{parent}: no ns {prefix}"))?,
            local.to_owned(),
        );
        let mut ranks: Option<Ranks> = None;
        for schemas in &sources {
            for steps in schemas.parent_steps(&key)? {
                ranks = Some(match ranks {
                    None => steps,
                    Some(base) => merge(base, &steps),
                });
            }
        }
        let ranks = ranks.ok_or_else(|| format!("{parent}: no schema type writes it; give an overlay"))?;
        let mut written = Vec::new();
        for step in ranks {
            let mut names = Vec::new();
            for (uri, local) in step {
                let p = prefix_of(&uri).ok_or_else(|| format!("{parent}: no ns line for {uri}"))?;
                names.push(format!("{p}:{local}"));
            }
            written.push(names);
        }
        out.push((parent.clone(), written, None));
    }
    Ok(out)
}

fn emit(d: &Description, orders: &[(String, Vec<Vec<String>>, Option<String>)]) -> String {
    let mut out = String::new();
    out.push_str("// @generated by `cargo xtask codegen` from schema/orders.list and the schemas in xsd/;\n");
    out.push_str("// do not edit.\n\n");
    out.push_str("//! The schemas' child orders (xtask/src/orders.rs): for each parent, the steps its children\n");
    out.push_str("//! go in, each step the names (prefixed as [`NAMESPACES`] writes them) that share it.\n\n");
    out.push_str("/// A parent's steps, in order; the names of one step share it.\n");
    out.push_str("pub type Steps = &'static [&'static [&'static str]];\n\n");
    out.push_str("/// Each prefix and its namespace, in order: the first prefix listed for a namespace is the one\n");
    out.push_str("/// its names are written with.\n");
    out.push_str("pub const NAMESPACES: &[(&str, &str)] = &[\n");
    for (prefix, uri) in &d.namespaces {
        let _ = writeln!(out, "    ({prefix:?}, {uri:?}),");
    }
    out.push_str("];\n\n");
    out.push_str("/// Each parent and its steps, in the description's order (an overlay's cause noted).\n");
    out.push_str("pub const ORDERS: &[(&str, Steps)] = &[\n");
    for (parent, steps, cause) in orders {
        if let Some(cause) = cause {
            let _ = writeln!(out, "    // overlay: {cause}");
        }
        let _ = write!(out, "    ({parent:?}, &[");
        for (k, step) in steps.iter().enumerate() {
            if k > 0 {
                out.push_str(", ");
            }
            out.push_str("&[");
            out.push_str(&step.iter().map(|n| format!("{n:?}")).collect::<Vec<_>>().join(", "));
            out.push(']');
        }
        out.push_str("]),\n");
    }
    out.push_str("];\n");
    out
}

/// Regenerate the orders; with `check`, report whether the checked-in module differs.
pub fn run(root: &Path, check: bool) -> Result<Option<String>, String> {
    let source = std::fs::read_to_string(root.join(DESCRIPTION)).map_err(|e| format!("{DESCRIPTION}: {e}"))?;
    let d = parse(&source)?;
    let generated = emit(&d, &generate_orders(root, &d)?);
    let path = root.join(OUTPUT);
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if current == generated {
        return Ok(None);
    }
    if !check {
        std::fs::write(&path, generated).map_err(|e| e.to_string())?;
    }
    Ok(Some(OUTPUT.to_owned()))
}
