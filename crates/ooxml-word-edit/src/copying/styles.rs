//! Owns: the styles a copy's blocks name, brought over as docx-agent's importer brings
//! them (`Importer.style`, `_formatting`): under `use_destination` a style the destination
//! has by the same name and kind (or, for the source's default style, the destination's
//! default of that kind) is the destination's; one it lacks is imported -- the source's
//! definition under a free id (`_free_style_id`), without its `w:default` and `w:rsid`,
//! what it is based on, its next and linked styles brought the same way first, a paragraph
//! style based in the destination given the source's document defaults where the
//! destination's differ (`_fold_defaults`) -- and appended to the destination's style
//! sheet (made when it has none). `style_map` names, by source style name, the
//! destination style a source style becomes; `unmapped: body` makes a style neither mapped
//! nor in the destination Word's built-in one, else the destination's body style
//! (`body_style`), instead of importing it. `keep_source` and `merge` flatten the source's
//! formatting into direct formatting ([`super::flatten`]).
//!
//! An imported style's lists are brought as the blocks' are ([`super::lists`]).
//!
//! Must not: write but into the edit's copy of the style sheet, or through
//! [`crate::builtin::ensure`] with that copy put back first.

use super::canonical::canonical;
use super::cascade::{tag_of, Formatting, Props, Tag};
use super::lists::ListMaps;
use super::theme::{theme_part, Themes};
use super::{ListPolicy, StylePolicy, UnmappedPolicy};
use crate::doc::{Key, Shared};
use crate::names::W;
use crate::placing::Place;
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::styles::REL_STYLES;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_base::pyfmt::repr_str;
use ooxml_word_model::docx::{casefold, style_name, Docx};
use ooxml_xml::{Element, QName};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A style's `w:type` as docx-agent reads it (`None` where absent).
pub(super) fn kind_of(style: &Element) -> Option<String> {
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

/// The `w:val` name.
fn w_val() -> ooxml_xml::Name {
    Arc::new(QName::new(Some("w"), "val", Some(W)))
}

/// The destination's styles as docx-agent's `_Styles` holds them: by id (first place, last
/// value), by kind and name (the first of each), and what each is.
#[derive(Default)]
pub(super) struct Sheet {
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

/// How a copy brings styles: the policy, the map and what an unmapped style becomes.
#[derive(Debug, Clone)]
pub(crate) struct Policies {
    /// Which styles the copies keep.
    pub(crate) styles: StylePolicy,
    /// Source style name (casefolded) -> destination style name.
    pub(crate) names: HashMap<String, String>,
    /// What a style neither mapped nor in the destination becomes.
    pub(crate) unmapped: UnmappedPolicy,
    /// Whether a copied list continues the list before the place.
    pub(crate) lists: ListPolicy,
}

/// The styles of one copy: the maps it keeps and the styles it imported.
pub(crate) struct Styles<'s> {
    pub(super) source: &'s Docx,
    pub(super) policies: Policies,
    dest: Sheet,
    /// The source's formatting cascade.
    pub(super) cascade: Formatting,
    /// The destination's, as the copy began.
    dest_cascade: Formatting,
    pub(super) themes: Themes,
    /// The destination's styles part (`None`: it has none yet).
    pub(super) part: Option<String>,
    /// Source style id -> destination style id (`None`: the default, no reference).
    map: HashMap<String, Option<String>>,
    /// The destination's body style, once read.
    body: Option<Option<String>>,
    /// The lists brought.
    pub(super) lists: ListMaps,
    shared: Arc<Shared>,
    /// What was imported or added, in order.
    pub(crate) warnings: Vec<String>,
}

impl<'s> Styles<'s> {
    /// The styles of a copy from `source` into the document `sink` edits, its style sheet
    /// and theme read (their keys declared).
    ///
    /// # Errors
    /// When a part is not XML; [`EditError::Unsupported`] for a theme docx-agent cannot
    /// read.
    pub(crate) fn new(
        sink: &mut EditSink<'_, '_>,
        source: &'s Docx,
        policies: Policies,
        place: &Place,
    ) -> Result<Styles<'s>> {
        let docx = sink.docx()?;
        let part = ooxml_word_model::parts::related(sink.doc().package(), &docx.main, REL_STYLES)?;
        if let Some(part) = &part {
            sink.read(Key::Part(Arc::from(part.as_str())));
        }
        if let Some(theme) = theme_part(&docx)? {
            sink.read(Key::Part(Arc::from(theme.as_str())));
        }
        let themes = Themes::read(source, &docx)?
            .ok_or_else(|| EditError::Unsupported("a theme whose colours docx-agent cannot read".to_owned()))?;
        Ok(Styles {
            source,
            lists: ListMaps::new(policies.lists, place.clone()),
            policies,
            dest: Sheet::of(&docx),
            cascade: Formatting::of(source),
            dest_cascade: Formatting::of(&docx),
            themes,
            part,
            map: HashMap::new(),
            body: None,
            shared: sink.doc().shared_handle(),
            warnings: Vec::new(),
        })
    }

    /// The child orders insertions follow.
    pub(super) fn orders(&self) -> &Orders {
        self.shared.facts.orders()
    }

    /// The destination's base formatting: its defaults and default paragraph style.
    pub(super) fn bases(&self) -> (Props, Props) {
        (
            self.dest_cascade.paragraph(None, None),
            self.dest_cascade.run(None, None, None),
        )
    }

    /// `element`'s styles brought over (`_formatting`).
    ///
    /// # Errors
    /// What [`Styles::style`] refuses or leaves to the host.
    pub(crate) fn bring(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        space: &mut Space,
        work: &mut Work,
        element: Id,
    ) -> Result<()> {
        if self.policies.styles != StylePolicy::UseDestination {
            return self.flatten_block(sink, space, work, element);
        }
        for node in work.iter(element, W, &["pStyle", "rStyle", "tblStyle"]) {
            let kind = match work.name(node).map(|n| n.local()) {
                Some("rStyle") => "character",
                Some("tblStyle") => "table",
                _ => "paragraph",
            };
            self.point(sink, space, work, node, kind)?;
        }
        Ok(())
    }

    /// A style reference pointed at the destination's style, or removed (`_set_or_drop`).
    pub(super) fn point(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        space: &mut Space,
        work: &mut Work,
        node: Id,
        kind: &str,
    ) -> Result<()> {
        let value = work.get(node, Some(W), "val").map(str::to_owned);
        let mapped = match value.as_deref() {
            None | Some("") => value.clone(),
            Some(v) => self.style(sink, space, v, Some(kind))?,
        };
        match mapped {
            None => work.remove(node),
            Some(mapped) if Some(&mapped) != value.as_ref() => work.set(node, w_val(), &mapped),
            Some(_) => {}
        }
        Ok(())
    }

    /// The destination's id for the source style `id` (`Importer.style`): as `style_map`
    /// names it, the destination's own by name, under `unmapped: body` a built-in or the
    /// body style, else the source's definition imported.
    pub(super) fn style(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        space: &mut Space,
        id: &str,
        kind: Option<&str>,
    ) -> Result<Option<String>> {
        if let Some(mapped) = self.map.get(id) {
            return Ok(mapped.clone());
        }
        let Some(node) = self.source.style(id).cloned() else {
            return Ok(Some(id.to_owned()));
        };
        let name = style_name(&node).filter(|n| !n.is_empty()).unwrap_or(id).to_owned();
        let kind = kind_of(&node).filter(|k| !k.is_empty()).or(kind.map(str::to_owned));
        if let Some(wanted) = self.policies.names.get(&casefold(&name)).cloned() {
            let Some(target) = self.destination_style(sink, space, &wanted, kind.as_deref())? else {
                return Err(EditError::Refused(format!(
                    "style_map: the destination has no {} style {} (and Word no built-in one by that name)",
                    kind.as_deref().unwrap_or("None"),
                    repr_str(&wanted)
                )));
            };
            self.map.insert(id.to_owned(), Some(target.clone()));
            return Ok(Some(target));
        }
        let mut existing = self.dest.by_name.get(&(kind.clone(), casefold(&name))).cloned();
        if existing.is_none() && is_default(node.attribute(Some(W), "default")) {
            existing = self.dest.default_of(kind.as_deref()).map(str::to_owned);
        }
        if let Some(existing) = existing {
            self.map.insert(id.to_owned(), Some(existing.clone()));
            return Ok(Some(existing));
        }
        if self.policies.unmapped == UnmappedPolicy::Body {
            return self.unmapped(sink, space, id, &name, kind.as_deref());
        }
        self.import(sink, space, id, &name, &node, kind.as_deref()).map(Some)
    }

    /// Under `unmapped: body`, a style neither mapped nor in the destination by name: Word's
    /// built-in one, else the body style (a paragraph style) or none.
    fn unmapped(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        space: &mut Space,
        id: &str,
        name: &str,
        kind: Option<&str>,
    ) -> Result<Option<String>> {
        let builtin = match kind {
            Some("paragraph" | "character" | "table") => self.destination_style(sink, space, name, kind)?,
            _ => None,
        };
        if let Some(builtin) = builtin {
            self.map.insert(id.to_owned(), Some(builtin.clone()));
            return Ok(Some(builtin));
        }
        let paragraph = kind == Some("paragraph");
        let target = if paragraph { self.body_style(sink, space)? } else { None };
        self.map.insert(id.to_owned(), target.clone());
        self.warnings.push(format!(
            "style {} not imported: {}",
            repr_str(name),
            if paragraph {
                "the destination's body style"
            } else {
                "dropped"
            }
        ));
        Ok(target)
    }

    /// The id of the destination's `kind` style `name` (a built-in one it lacks added as
    /// Word writes it), or `None` when it has none and Word no such built-in style
    /// (`_destination_style`).
    ///
    /// # Errors
    /// When a part is not XML.
    pub(super) fn destination_style(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        space: &mut Space,
        name: &str,
        kind: Option<&str>,
    ) -> Result<Option<String>> {
        let Some(kind) = kind else {
            return Err(EditError::Unsupported("a copied style without a kind".to_owned()));
        };
        if let Some(part) = &self.part {
            space.put_part(sink, part)?;
        }
        let docx = sink.docx()?;
        if let Some(found) = docx.find_style(name, kind) {
            return Ok(Some(found.attribute(Some(W), "styleId").unwrap_or("").to_owned()));
        }
        match crate::builtin::ensure(sink, name, kind) {
            Ok(id) => {
                let docx = sink.docx()?;
                self.dest = Sheet::of(&docx);
                self.part = ooxml_word_model::parts::related(sink.doc().package(), &docx.main, REL_STYLES)?;
                self.warnings
                    .push(format!("style {} added as Word writes it", repr_str(name)));
                Ok(Some(id))
            }
            Err(EditError::Refused(_)) => Ok(None),
            Err(other) => Err(other),
        }
    }

    /// The destination's body style (`body_style`): the paragraph style most of its body's
    /// plain paragraphs use (no heading, no list, some text), `None` for its default.
    fn body_style(&mut self, sink: &mut EditSink<'_, '_>, space: &mut Space) -> Result<Option<String>> {
        if let Some(body) = &self.body {
            return Ok(body.clone());
        }
        if let Some(part) = &self.part {
            space.put_part(sink, part)?;
        }
        let docx = sink.docx()?;
        // which style the body's paragraphs use: any of them
        sink.read(Key::Part(Arc::from(docx.main.as_str())));
        let best = body_style_of(sink, &docx)?;
        self.body = Some(best.clone());
        Ok(best)
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
        let part = match self.part.clone() {
            Some(part) => part,
            None => {
                let part = crate::builtin::styles_part(sink)?;
                self.part = Some(part.clone());
                part
            }
        };
        let new_id = self.free_id(id, name);
        self.map.insert(id.to_owned(), Some(new_id.clone()));
        // built apart and appended last, as docx-agent does: what it brings first (the
        // styles it names, their lists) may put the style sheet back meanwhile
        let (mut work, copy) = detached_copy(node)?;
        work.set(copy, Arc::new(QName::new(Some("w"), "styleId", Some(W))), &new_id);
        work.remove_attr(copy, Some(W), "default");
        for child in work.children(copy).to_vec() {
            if work.is(child, W, "rsid") {
                work.remove(child);
            }
        }
        for tag in ["basedOn", "next", "link"] {
            let Some(reference) = work.find(copy, W, tag) else {
                continue;
            };
            let value = work.get(reference, Some(W), "val").map(str::to_owned);
            let target = value.as_deref().and_then(|v| self.source.style(v)).cloned();
            let (Some(value), Some(target)) = (value, target) else {
                work.remove(reference);
                continue;
            };
            let mapped = self
                .style(sink, space, &value, kind_of(&target).as_deref())?
                .ok_or_else(|| EditError::Unsupported("a copied style based on no style".to_owned()))?;
            work.set(reference, w_val(), &mapped);
        }
        self.style_lists(sink, space, &mut work, copy)?;
        let shared = Arc::clone(&self.shared);
        if kind == Some("paragraph") && self.based_in_destination(&work, copy) {
            self.fold_defaults(&mut work, copy, shared.facts.orders())?;
        }
        let built = work.build_element(copy).ok_or_else(|| EditError::Path(part.clone()))?;
        let styles = styles_work(sink, space, &part)?;
        let adopted = styles.adopt(&built).ok_or_else(|| EditError::Path(part.clone()))?;
        let root = styles.root();
        styles.append(root, adopted);
        let own_kind = built.attribute(Some(W), "type").map(str::to_owned);
        self.dest.add(&new_id, name_key(node), own_kind, false);
        space.mark_dirty(&part);
        self.warnings
            .push(format!("style {} imported from the source", repr_str(name)));
        Ok(new_id)
    }

    /// Whether the imported style `copy` is based on none or on a destination style.
    fn based_in_destination(&self, work: &Work, copy: Id) -> bool {
        match work.find(copy, W, "basedOn") {
            None => true,
            Some(based) => work
                .get(based, Some(W), "val")
                .is_some_and(|v| self.dest.by_id.contains_key(v)),
        }
    }

    /// The source's document defaults where the destination's differ (compared in their
    /// trees, as lxml's inclusive canonical form compares them), into the paragraph style
    /// `copy` imported, where it does not state them itself (`_fold_defaults`).
    fn fold_defaults(&self, work: &mut Work, copy: Id, orders: &Orders) -> Result<()> {
        for tag in ["rPr", "pPr"] {
            let Some((source, mut scope)) = self.cascade.defaults(tag) else {
                continue;
            };
            scope.extend_from_slice(source.namespaces());
            let mut ours: Vec<(Tag, String)> = Vec::new();
            if let Some((dest, mut theirs)) = self.dest_cascade.defaults(tag) {
                theirs.extend_from_slice(dest.namespaces());
                for child in dest.child_elements() {
                    let form = canonical(child, &theirs);
                    match ours.iter_mut().find(|(t, _)| *t == tag_of(child)) {
                        Some(entry) => entry.1 = form,
                        None => ours.push((tag_of(child), form)),
                    }
                }
            }
            let mut container = work.find(copy, W, tag);
            for child in source.child_elements() {
                let child_tag = tag_of(child);
                if ours
                    .iter()
                    .any(|(t, form)| *t == child_tag && *form == canonical(child, &scope))
                {
                    continue;
                }
                let stated = container.is_some_and(|c| {
                    work.children(c).iter().any(|n| {
                        work.name(*n).is_some_and(|name| {
                            name.namespace() == child.name().namespace() && name.local() == child.name().local()
                        })
                    })
                });
                if stated {
                    continue;
                }
                let into = match container {
                    Some(c) => c,
                    None => {
                        let made = crate::review::xml::make(work, tag, &[]);
                        crate::marks::insert_in_order(work, copy, made, orders);
                        container = Some(made);
                        made
                    }
                };
                let value = self.themes.concrete(child);
                let adopted = work.adopt_copy(&value).ok_or_else(|| EditError::Path(tag.to_owned()))?;
                crate::marks::insert_in_order(work, into, adopted, orders);
            }
        }
        Ok(())
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

/// `element` (another document's) as lxml's `copy.deepcopy` makes it, in a work of its
/// own: detached, declaring the namespaces its names use.
///
/// # Errors
/// When the element cannot be read.
pub(super) fn detached_copy(element: &Arc<Element>) -> Result<(Work, Id)> {
    let mut work = Work::of(element, Vec::new());
    let copy = work
        .adopt_copy(element)
        .ok_or_else(|| EditError::Path(element.name().local().to_owned()))?;
    Ok((work, copy))
}

/// The edit's copy of the style sheet `part`, made on first use.
fn styles_work<'w>(sink: &EditSink<'_, '_>, space: &'w mut Space, part: &str) -> Result<&'w mut Work> {
    let held = sink.doc().clone();
    space
        .work(&held, part)?
        .ok_or_else(|| EditError::NoPart(part.to_owned()))
}

/// The paragraph style most of `docx`'s body's plain paragraphs use, `None` for its
/// default (`body_style`'s count).
fn body_style_of(sink: &EditSink<'_, '_>, docx: &Docx) -> Result<Option<String>> {
    let sheet = Formatting::of(docx);
    let tree = sink.doc().tree(&docx.main)?;
    let mut counts: Vec<(Option<String>, usize)> = Vec::new();
    let body = tree.as_ref().and_then(|t| t.root().find(Some(W), "body").cloned());
    for paragraph in body
        .iter()
        .flat_map(|b| b.child_elements())
        .filter(|p| p.name().is(Some(W), "p"))
    {
        let properties = paragraph.find(Some(W), "pPr");
        if properties.is_some_and(|p| p.find(Some(W), "numPr").is_some()) {
            continue;
        }
        let style = properties
            .and_then(|p| p.find(Some(W), "pStyle"))
            .and_then(|s| s.attribute(Some(W), "val"))
            .map(str::to_owned);
        if let Some(id) = &style {
            let name = sheet.name(id).unwrap_or("");
            let structural = super::flatten::is_heading(name)
                || ["title", "subtitle"].contains(&casefold(name).as_str())
                || super::flatten::is_list(name)
                || sheet.chain(Some(id), "paragraph").iter().any(|level| {
                    level
                        .find(Some(W), "pPr")
                        .is_some_and(|p| p.find(Some(W), "outlineLvl").is_some() || p.find(Some(W), "numPr").is_some())
                });
            if structural {
                continue;
            }
        }
        if super::flatten::t_text(paragraph).trim().is_empty() {
            continue;
        }
        match counts.iter_mut().find(|(s, _)| *s == style) {
            Some(entry) => entry.1 += 1,
            None => counts.push((style, 1)),
        }
    }
    let mut best: Option<(Option<String>, usize)> = None;
    for (style, count) in counts {
        if best.as_ref().is_none_or(|(_, n)| count > *n) {
            best = Some((style, count));
        }
    }
    let best = best.and_then(|(s, _)| s);
    let default = sheet.default("paragraph").and_then(|d| d.attribute(Some(W), "styleId"));
    Ok(best.filter(|b| Some(b.as_str()) != default))
}
