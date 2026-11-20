//! Owns: effective paragraph and run formatting in one document at the XML level, as
//! docx-agent's importer reads it (`_Styles`, `_merge_into`, `_Effective`): the document
//! defaults, the paragraph style's chain, the character style's chain and the direct
//! formatting folded one level after another -- later wins, the attributes of fonts,
//! languages, spacing and indents merged one by one, toggle properties flipping along each
//! chain.
//!
//! Each value is a detached copy (lxml's `copy.deepcopy`), an [`Element`] here; the
//! properties are kept in the order Python's dictionary keeps them (first insertion,
//! last value).
//!
//! Must not: read anything but the style sheet it is given and the elements passed in.

use crate::names::W;
use crate::styles::REL_STYLES;
use ooxml_word_model::docx::{style_name, Docx};
use ooxml_xml::{Atom, Attribute, Element, Name, NamespaceDecl, QName};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Run properties that toggle along a style hierarchy (ECMA-376 17.7.3).
pub(crate) const TOGGLES: [&str; 13] = [
    "b",
    "bCs",
    "i",
    "iCs",
    "caps",
    "smallCaps",
    "strike",
    "dstrike",
    "outline",
    "shadow",
    "emboss",
    "imprint",
    "vanish",
];
/// Properties whose attributes merge level by level, rather than the element replacing.
const MERGED: [&str; 4] = ["rFonts", "lang", "spacing", "ind"];
/// Paragraph properties that are not formatting.
const NOT_FORMATTING: [&str; 6] = ["pStyle", "rPr", "sectPr", "pPrChange", "numPr", "rStyle"];
/// A theme font attribute and the plain one it stands for.
pub(crate) const THEME_FONTS: [(&str, &str); 4] = [
    ("asciiTheme", "ascii"),
    ("hAnsiTheme", "hAnsi"),
    ("eastAsiaTheme", "eastAsia"),
    ("cstheme", "cs"),
];

/// An element's name as a dictionary key: its namespace and local name.
pub(crate) type Tag = (Option<String>, String);

/// The tag of `element`.
pub(crate) fn tag_of(element: &Element) -> Tag {
    let name = element.name();
    (name.namespace().map(str::to_owned), name.local().to_owned())
}

fn is_w(tag: &Tag, locals: &[&str]) -> bool {
    tag.0.as_deref() == Some(W) && locals.contains(&tag.1.as_str())
}

/// Whether `tag` is a toggle property.
pub(crate) fn is_toggle(tag: &Tag) -> bool {
    is_w(tag, &TOGGLES)
}

/// docx-agent's `_on`: a property's `w:val` read as on (none: on).
pub(crate) fn on(element: &Element) -> bool {
    let value = element.attribute(Some(W), "val").unwrap_or("true").to_lowercase();
    !matches!(value.as_str(), "0" | "false" | "off" | "none")
}

/// Properties by tag, in the order a Python dictionary keeps them.
#[derive(Debug, Clone, Default)]
pub(crate) struct Props(Vec<(Tag, Arc<Element>)>);

impl Props {
    pub(crate) fn get(&self, tag: &Tag) -> Option<&Arc<Element>> {
        self.0.iter().find(|(t, _)| t == tag).map(|(_, e)| e)
    }

    pub(crate) fn set(&mut self, tag: Tag, value: Arc<Element>) {
        match self.0.iter_mut().find(|(t, _)| *t == tag) {
            Some(slot) => slot.1 = value,
            None => self.0.push((tag, value)),
        }
    }

    fn pop(&mut self, tag: &Tag) -> Option<Arc<Element>> {
        let at = self.0.iter().position(|(t, _)| t == tag)?;
        Some(self.0.remove(at).1)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &(Tag, Arc<Element>)> {
        self.0.iter()
    }
}

/// `element` with the attribute `name` set to `value`: in its place when it has it, else
/// after the others (lxml's `set`).
pub(crate) fn with_attribute(element: &Element, name: &Name, value: &str) -> Element {
    let mut attributes = element.attributes().to_vec();
    let value = Atom::from(value);
    match attributes
        .iter_mut()
        .find(|a| a.name.namespace() == name.namespace() && a.name.local() == name.local())
    {
        Some(a) => a.value = value,
        None => attributes.push(Attribute {
            name: Arc::clone(name),
            value,
        }),
    }
    rebuilt(element, attributes)
}

/// `element` without the `w:` attributes `locals`.
pub(crate) fn without_w(element: &Element, locals: &[&str]) -> Element {
    let attributes = element
        .attributes()
        .iter()
        .filter(|a| !(a.name.namespace() == Some(W) && locals.contains(&a.name.local())))
        .cloned()
        .collect();
    rebuilt(element, attributes)
}

fn rebuilt(element: &Element, attributes: Vec<Attribute>) -> Element {
    Element::new(
        element.name().clone(),
        element.namespaces().to_vec(),
        attributes,
        element.children().iter().cloned().collect(),
    )
}

/// Fold one level's properties into `accumulated` (`_merge_into`); toggles go to
/// `toggles` when it is given, flipping.
fn merge_into(accumulated: &mut Props, node: Option<&Element>, mut toggles: Option<&mut HashMap<Tag, bool>>) {
    let Some(node) = node else { return };
    for child in node.child_elements() {
        let tag = tag_of(child);
        if is_w(&tag, &NOT_FORMATTING) || tag.1.ends_with("Change") {
            continue;
        }
        if let Some(flips) = toggles.as_deref_mut().filter(|_| is_toggle(&tag)) {
            let was = flips.get(&tag).copied().unwrap_or(false);
            flips.insert(tag, was ^ on(child));
            continue;
        }
        match accumulated.get(&tag).filter(|_| is_w(&tag, &MERGED)) {
            Some(before) => {
                let mut merged: Element = (**before).clone();
                for attribute in child.attributes() {
                    let local = attribute.name.local();
                    if tag.1 == "rFonts" {
                        // a face named outright replaces the theme's, and the other way round
                        for (theme, plain) in THEME_FONTS {
                            if local == plain {
                                merged = without_w(&merged, &[theme]);
                            } else if local == theme {
                                merged = without_w(&merged, &[plain]);
                            }
                        }
                    }
                    if tag.1 == "ind" && matches!(local, "hanging" | "firstLine") {
                        merged = without_w(&merged, &["hanging", "firstLine"]);
                    }
                    merged = with_attribute(&merged, &attribute.name, &attribute.value);
                }
                accumulated.set(tag, Arc::new(merged));
            }
            None => accumulated.set(tag, Arc::clone(child)),
        }
    }
}

/// A new `w:<local>` element (`make`), `w:val="0"` when off.
fn toggle(local: &str, value: bool) -> Element {
    let name = Arc::new(QName::new(Some("w"), local, Some(W)));
    let attributes = if value {
        Vec::new()
    } else {
        vec![Attribute {
            name: Arc::new(QName::new(Some("w"), "val", Some(W))),
            value: Atom::from("0"),
        }]
    };
    Element::new(
        name,
        vec![NamespaceDecl {
            prefix: Some(Atom::from("w")),
            uri: Atom::from(W),
        }],
        attributes,
        Vec::new(),
    )
}

/// One document's styles as the importer's `_Styles` holds them: by id (first place, last
/// value), and its document defaults.
pub(crate) struct Formatting {
    order: Vec<String>,
    by_id: HashMap<String, Arc<Element>>,
    root: Option<Arc<Element>>,
}

impl Formatting {
    /// `docx`'s styles.
    pub(crate) fn of(docx: &Docx) -> Formatting {
        let mut order = Vec::new();
        let mut by_id = HashMap::new();
        for style in docx.styles() {
            let id = style.attribute(Some(W), "styleId").unwrap_or("").to_owned();
            if !by_id.contains_key(&id) {
                order.push(id.clone());
            }
            by_id.insert(id, Arc::clone(style));
        }
        Formatting {
            order,
            by_id,
            root: styles_root(docx),
        }
    }

    /// The `w:name` of the style `id` names.
    pub(crate) fn name(&self, id: &str) -> Option<&str> {
        self.by_id.get(id).and_then(|s| style_name(s))
    }

    /// The first style of `kind` marked default (`1`, `true`, `on`).
    pub(crate) fn default(&self, kind: &str) -> Option<&Arc<Element>> {
        self.order.iter().filter_map(|id| self.by_id.get(id)).find(|s| {
            s.attribute(Some(W), "type") == Some(kind)
                && matches!(s.attribute(Some(W), "default"), Some("1" | "true" | "on"))
        })
    }

    /// The style and what it is based on, root first (the default of `kind` when none).
    pub(crate) fn chain(&self, id: Option<&str>, kind: &str) -> Vec<Arc<Element>> {
        let mut node = match id.filter(|i| !i.is_empty()) {
            Some(id) => self.by_id.get(id),
            None => self.default(kind),
        };
        let mut out: Vec<Arc<Element>> = Vec::new();
        let mut seen: HashSet<usize> = HashSet::new();
        while let Some(style) = node.filter(|s| seen.insert(Arc::as_ptr(s) as usize)) {
            out.push(Arc::clone(style));
            node = style
                .find(Some(W), "basedOn")
                .and_then(|b| b.attribute(Some(W), "val"))
                .and_then(|v| self.by_id.get(v));
        }
        out.reverse();
        out
    }

    /// The document's default `pPr` or `rPr` (`docDefaults/{tag}Default/{tag}`), with the
    /// declarations in scope above it.
    pub(crate) fn defaults(&self, tag: &str) -> Option<(Arc<Element>, Vec<NamespaceDecl>)> {
        let root = self.root.as_ref()?;
        let mut scope = root.namespaces().to_vec();
        let mut node = root;
        for step in ["docDefaults", &format!("{tag}Default")] {
            node = node.find(Some(W), step)?;
            scope.extend_from_slice(node.namespaces());
        }
        Some((Arc::clone(node.find(Some(W), tag)?), scope))
    }

    /// Effective paragraph formatting (`_Effective.paragraph`).
    pub(crate) fn paragraph(&self, style: Option<&str>, direct: Option<&Element>) -> Props {
        let mut out = Props::default();
        merge_into(&mut out, self.defaults("pPr").as_ref().map(|(d, _)| &**d), None);
        for node in self.chain(style, "paragraph") {
            merge_into(&mut out, node.find(Some(W), "pPr").map(|p| &**p), None);
        }
        merge_into(&mut out, direct, None);
        out
    }

    /// Effective run formatting (`_Effective.run`).
    pub(crate) fn run(&self, paragraph: Option<&str>, character: Option<&str>, direct: Option<&Element>) -> Props {
        let mut out = Props::default();
        merge_into(&mut out, self.defaults("rPr").as_ref().map(|(d, _)| &**d), None);
        let toggled: Vec<Tag> = out.iter().map(|(t, _)| t.clone()).filter(is_toggle).collect();
        let mut base: HashMap<Tag, bool> = HashMap::new();
        for tag in toggled {
            if let Some(node) = out.pop(&tag) {
                base.insert(tag, on(&node));
            }
        }
        let mut levels: Vec<HashMap<Tag, bool>> = Vec::new();
        let chains = [
            self.chain(paragraph, "paragraph"),
            match character.filter(|c| !c.is_empty()) {
                Some(id) => self.chain(Some(id), "character"),
                None => Vec::new(),
            },
        ];
        for chain in chains {
            let mut flips: HashMap<Tag, bool> = HashMap::new();
            for node in chain {
                let mut level = HashMap::new();
                merge_into(&mut out, node.find(Some(W), "rPr").map(|p| &**p), Some(&mut level));
                flips.extend(level);
            }
            levels.push(flips);
        }
        for local in TOGGLES {
            let tag: Tag = (Some(W.to_owned()), local.to_owned());
            let value = if levels.iter().any(|l| l.contains_key(&tag)) {
                levels
                    .iter()
                    .fold(false, |v, l| v ^ l.get(&tag).copied().unwrap_or(false))
            } else if let Some(value) = base.get(&tag) {
                *value
            } else {
                continue;
            };
            out.set(tag, Arc::new(toggle(local, value)));
        }
        if let Some(direct) = direct {
            for child in direct.child_elements() {
                let tag = tag_of(child);
                if is_toggle(&tag) {
                    out.set(tag, Arc::clone(child));
                }
            }
            merge_into(&mut out, Some(direct), None);
        }
        out
    }
}

/// The root of `docx`'s style sheet.
pub(crate) fn styles_root(docx: &Docx) -> Option<Arc<Element>> {
    let part = ooxml_word_model::parts::related(&docx.package, &docx.main, REL_STYLES).ok()??;
    let tree = docx.package.tree(&part).ok()??;
    Some(Arc::clone(tree.root()))
}
