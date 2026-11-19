//! Owns: the styles a copy's blocks name, brought over as docx-agent's importer brings
//! them under its default policy (`Importer.style`, `use_destination`): a style the
//! destination has by the same name and kind (or, for the source's default style, the
//! destination's default of that kind) is the destination's; one it lacks is imported --
//! the source's definition under a free id (`_free_style_id`), without its `w:default`
//! and `w:rsid`, what it is based on, its next and linked styles brought the same way
//! first -- and appended to the destination's style sheet.
//!
//! Not ported yet ([`EditError::Unsupported`]: the host copies such blocks its own way): an
//! imported style naming a list, an imported paragraph style that would take the source's
//! document defaults (`_fold_defaults`), a destination without a styles part.
//!
//! Must not: write but into the edit's copy of the style sheet.

use crate::doc::Key;
use crate::names::W;
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::styles::REL_STYLES;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_base::pyfmt::repr_str;
use ooxml_word_model::docx::{casefold, find_path, style_name, Docx};
use ooxml_xml::{Element, QName};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A style's `w:type` as docx-agent reads it (`None` where absent).
fn kind_of(style: &Element) -> Option<String> {
    style.attribute(Some(W), "type").map(str::to_owned)
}

/// The name a style is found by (`""` for a `w:name` without a value; `None`: no name).
fn name_key(style: &Element) -> Option<&str> {
    style
        .find(Some(W), "name")
        .map(|n| n.attribute(Some(W), "val").unwrap_or(""))
}

/// Whether a style is marked default as docx-agent's importer reads it (`1`, `true`, `on`
/// exactly).
fn is_default(value: Option<&str>) -> bool {
    matches!(value, Some("1" | "true" | "on"))
}

/// The destination's styles as docx-agent's `_Styles` holds them: by id (first place, last
/// value), by kind and name (the first of each), and what each is.
#[derive(Default)]
struct Sheet {
    /// Ids in the order first met.
    order: Vec<String>,
    /// By id: its kind and whether it is marked default.
    by_id: HashMap<String, (Option<String>, bool)>,
    /// By kind and casefolded name: the id.
    by_name: HashMap<(Option<String>, String), String>,
}

impl Sheet {
    fn of(docx: &Docx) -> Sheet {
        let mut sheet = Sheet::default();
        for style in docx.styles() {
            let default = is_default(style.attribute(Some(W), "default"));
            sheet.add(
                style.attribute(Some(W), "styleId").unwrap_or(""),
                name_key(style),
                kind_of(style),
                default,
            );
        }
        sheet
    }

    fn add(&mut self, id: &str, name: Option<&str>, kind: Option<String>, default: bool) {
        if !self.by_id.contains_key(id) {
            self.order.push(id.to_owned());
        }
        self.by_id.insert(id.to_owned(), (kind.clone(), default));
        if let Some(name) = name {
            self.by_name
                .entry((kind, casefold(name)))
                .or_insert_with(|| id.to_owned());
        }
    }

    /// The first style of `kind` marked default (`_Styles.default`).
    fn default_of(&self, kind: Option<&str>) -> Option<&str> {
        self.order
            .iter()
            .find(|id| {
                self.by_id
                    .get(*id)
                    .is_some_and(|(k, default)| k.as_deref() == kind && *default)
            })
            .map(String::as_str)
    }
}

/// The styles of one copy: the maps it keeps and the styles it imported.
pub(crate) struct Styles<'s> {
    source: &'s Docx,
    dest: Sheet,
    /// The destination's styles part (`None`: it has none).
    part: Option<String>,
    /// Source style id -> destination style id.
    map: HashMap<String, String>,
    /// What was imported, in order (`style 'X' imported from the source`).
    pub(crate) warnings: Vec<String>,
}

impl<'s> Styles<'s> {
    /// The styles of a copy from `source` into the document `sink` edits, its style sheet
    /// read (its key declared).
    ///
    /// # Errors
    /// When a part is not XML.
    pub(crate) fn new(sink: &mut EditSink<'_, '_>, source: &'s Docx) -> Result<Styles<'s>> {
        let docx = sink.docx()?;
        let part = ooxml_word_model::parts::related(sink.doc().package(), &docx.main, REL_STYLES)?;
        if let Some(part) = &part {
            sink.read(Key::Part(Arc::from(part.as_str())));
        }
        Ok(Styles {
            source,
            dest: Sheet::of(&docx),
            part,
            map: HashMap::new(),
            warnings: Vec::new(),
        })
    }

    /// Every style reference in `element` (`w:pStyle`, `w:rStyle`, `w:tblStyle`) pointed at
    /// the destination's style (`_formatting` under `use_destination`); a reference without
    /// a value goes.
    ///
    /// # Errors
    /// What [`Styles::style`] leaves to the host.
    pub(crate) fn bring(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        space: &mut Space,
        work: &mut Work,
        element: Id,
    ) -> Result<()> {
        for node in work.iter(element, W, &["pStyle", "rStyle", "tblStyle"]) {
            let kind = match work.name(node).map(|n| n.local()) {
                Some("rStyle") => "character",
                Some("tblStyle") => "table",
                _ => "paragraph",
            };
            let Some(value) = work.get(node, Some(W), "val").map(str::to_owned) else {
                work.remove(node);
                continue;
            };
            let mapped = self.style(sink, space, &value, Some(kind))?;
            if mapped != value {
                work.set(node, Arc::new(QName::new(Some("w"), "val", Some(W))), &mapped);
            }
        }
        Ok(())
    }

    /// The destination's id for the source style `id` (`Importer.style`): the
    /// destination's own by name, else the source's definition imported.
    fn style(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        space: &mut Space,
        id: &str,
        kind: Option<&str>,
    ) -> Result<String> {
        if id.is_empty() {
            return Ok(String::new());
        }
        if let Some(mapped) = self.map.get(id) {
            return Ok(mapped.clone());
        }
        let Some(node) = self.source.style(id).cloned() else {
            return Ok(id.to_owned());
        };
        let name = style_name(&node).filter(|n| !n.is_empty()).unwrap_or(id).to_owned();
        let kind = kind_of(&node).filter(|k| !k.is_empty()).or(kind.map(str::to_owned));
        let mut existing = self.dest.by_name.get(&(kind.clone(), casefold(&name))).cloned();
        if existing.is_none() && is_default(node.attribute(Some(W), "default")) {
            existing = self.dest.default_of(kind.as_deref()).map(str::to_owned);
        }
        if let Some(existing) = existing {
            self.map.insert(id.to_owned(), existing.clone());
            return Ok(existing);
        }
        self.import(sink, space, id, &name, &node, kind.as_deref())
    }

    /// The source's style `node` (`id`, named `name`) imported under a free id.
    fn import(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        space: &mut Space,
        id: &str,
        name: &str,
        node: &Arc<Element>,
        kind: Option<&str>,
    ) -> Result<String> {
        let Some(part) = self.part.clone() else {
            return Err(EditError::Unsupported(
                "a style copied into a document without styles".to_owned(),
            ));
        };
        let new_id = self.free_id(id, name);
        self.map.insert(id.to_owned(), new_id.clone());
        let held = sink.doc().clone();
        let work = space
            .work(&held, &part)?
            .ok_or_else(|| EditError::NoPart(part.clone()))?;
        let adopted = work.adopt(node).ok_or_else(|| EditError::Path(part.clone()))?;
        let copy = work.deep_copy(adopted);
        work.set(copy, Arc::new(QName::new(Some("w"), "styleId", Some(W))), &new_id);
        work.remove_attr(copy, Some(W), "default");
        for child in work.children(copy).to_vec() {
            if work.is(child, W, "rsid") {
                work.remove(child);
            }
        }
        for tag in ["basedOn", "next", "link"] {
            let work = space.get_mut(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
            let Some(reference) = work.find(copy, W, tag) else {
                continue;
            };
            let value = work.get(reference, Some(W), "val").map(str::to_owned);
            let target = value.as_deref().and_then(|v| self.source.style(v)).cloned();
            let (Some(value), Some(target)) = (value, target) else {
                work.remove(reference);
                continue;
            };
            let mapped = self.style(sink, space, &value, kind_of(&target).as_deref())?;
            let work = space.get_mut(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
            work.set(reference, Arc::new(QName::new(Some("w"), "val", Some(W))), &mapped);
        }
        let work = space.get_mut(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
        if !work.iter(copy, W, &["numId"]).is_empty() {
            return Err(EditError::Unsupported("a copied style's list".to_owned()));
        }
        if kind == Some("paragraph") && self.folds_defaults(work, copy) {
            return Err(EditError::Unsupported(
                "a copied style taking the source's defaults".to_owned(),
            ));
        }
        let root = work.root();
        work.append(root, copy);
        let own_kind = work.get(copy, Some(W), "type").map(str::to_owned);
        self.dest.add(&new_id, name_key(node), own_kind, false);
        space.mark_dirty(&part);
        self.warnings
            .push(format!("style {} imported from the source", repr_str(name)));
        Ok(new_id)
    }

    /// Whether the source's document defaults would be written into the imported paragraph
    /// style `copy` (`_fold_defaults`, for a style based in the destination): a default it
    /// does not state itself. Read without comparing with the destination's defaults, so a
    /// style that would take none may be counted as taking some (the host copies it then).
    fn folds_defaults(&self, work: &Work, copy: Id) -> bool {
        let based = work
            .find(copy, W, "basedOn")
            .map(|b| work.get(b, Some(W), "val").unwrap_or("").to_owned());
        if based.is_some_and(|b| !self.dest.by_id.contains_key(&b)) {
            return false;
        }
        let Some(root) = styles_root(self.source) else {
            return false;
        };
        ["rPr", "pPr"].iter().any(|tag| {
            let default = format!("{tag}Default");
            let Some(defaults) = find_path(&root, &["docDefaults", &default, tag]) else {
                return false;
            };
            let container = work.find(copy, W, tag);
            defaults.child_elements().any(|child| {
                let stated = container.is_some_and(|c| {
                    work.children(c).iter().any(|n| {
                        work.name(*n).is_some_and(|name| {
                            name.namespace() == child.name().namespace() && name.local() == child.name().local()
                        })
                    })
                });
                !stated
            })
        })
    }

    /// A style id the destination does not use, in any case (`_free_style_id`): the source's,
    /// else the name's letters and digits numbered from 2.
    fn free_id(&self, id: &str, name: &str) -> String {
        let taken: HashSet<String> = self.dest.by_id.keys().map(|k| casefold(k)).collect();
        let stem: String = name.chars().filter(char::is_ascii_alphanumeric).collect();
        let stem = if stem.is_empty() { "Style" } else { stem.as_str() };
        let mut candidate = id.to_owned();
        let mut number = 1;
        while taken.contains(&casefold(&candidate)) {
            number += 1;
            candidate = format!("{stem}{number}");
        }
        candidate
    }
}

/// The root of `docx`'s style sheet.
fn styles_root(docx: &Docx) -> Option<Arc<Element>> {
    let part = ooxml_word_model::parts::related(&docx.package, &docx.main, REL_STYLES).ok()??;
    let tree = docx.package.tree(&part).ok()??;
    Some(Arc::clone(tree.root()))
}
