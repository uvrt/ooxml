//! Owns: the parts beside the version's text that a page's drawing reads (its pictures'
//! parts and media types, its drawings' charts, diagrams and the parts they relate to, the
//! pictures a shape's or VML object's markup names, the theme and settings a drawing or a
//! text effect is coloured from), each by its name and a digest of its content -- what a
//! comparison of two versions' pages digests beside the page's paint, so that a picture
//! replaced or a chart edited under the same relationship tells the page apart
//! ([`crate::digests`]); and the content digests, kept by each part's content
//! ([`PartIdentity`]) for as long as the engine's memo keeps them, so a part is digested
//! once across the versions that share it.
//!
//! Must not: paint, or read a part the version's model reads as its text (those are drawn
//! in the paint, and digested there).

use crate::version::LaidVersion;
use ooxml_base::{Digest, DigestKey};
use ooxml_derive::{CacheBudget, Memo, Weigh};
use ooxml_opc::{Package, PartIdentity};
use ooxml_word_layout::paint::drawings::DrawingSlot;
use ooxml_word_layout::paint::names::PageNames;
use ooxml_word_layout::paint::PagePaint;
use ooxml_word_model::model::Graphic;
use ooxml_word_model::parts::rel_type_is;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

/// What a drawing or a text effect is coloured from beside its own markup: the theme (its
/// colours, formats and fonts) and the settings (their colour map).
const COLOURED_FROM: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme",
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings",
];
/// The namespaces a relationship id is named in, in markup (`r:embed`, `r:id`, `r:link`).
const RELATIONSHIPS: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    "http://purl.oclc.org/ooxml/officeDocument/relationships",
];

/// One part a page's drawing reads: its name, its media type and its content's digest
/// (`None`: a part the package does not have).
pub(crate) type PartRead = (String, Option<String>, Option<Digest>);

#[derive(Clone, Copy)]
struct Content(Digest);

impl Weigh for Content {
    fn weight(&self) -> usize {
        std::mem::size_of::<Digest>()
    }
}

/// The parts' content digests an engine made, by each part's content, within a budget.
pub(crate) struct PartDigests(Mutex<Memo<PartIdentity, Content>>);

impl PartDigests {
    /// An empty memo reserving from `budget`.
    pub(crate) fn new(budget: Arc<dyn CacheBudget>) -> PartDigests {
        PartDigests(Mutex::new(Memo::new(budget)))
    }

    /// The digest of part `name`'s content in `package`, made where not kept.
    fn of(&self, package: &Package, name: &str, key: DigestKey) -> Option<Digest> {
        let identity = package.part(name)?.identity();
        let held = self.0.lock().unwrap_or_else(PoisonError::into_inner).get(&identity);
        if let Some(Content(digest)) = held {
            return Some(digest);
        }
        let digest = Digest::of(key, &package.read(name).ok().flatten()?);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(identity, Content(digest));
        Some(digest)
    }
}

impl std::fmt::Debug for PartDigests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PartDigests").finish_non_exhaustive()
    }
}

/// The parts beside the text that `paint` (a page of `version`) reads, in name order, each
/// with its media type and content digested under `key`.
pub(crate) fn parts_read(
    paint: &PagePaint,
    version: &LaidVersion,
    memo: &PartDigests,
    key: DigestKey,
) -> Vec<PartRead> {
    let Some(pictures) = version.pictures.as_ref() else {
        return Vec::new();
    };
    let package = pictures.package();
    let main = pictures.main_part();
    let names = PageNames::of_page(&version.layout.document.names, paint);
    let mut read = BTreeSet::new();
    let text_boxes = paint.text_boxes.iter();
    let pictures_drawn = paint
        .pictures
        .iter()
        .chain(text_boxes.clone().flat_map(|t| t.pictures.iter()));
    for d in pictures_drawn {
        if let Some(relationship) = d.relationship.as_deref() {
            let owner = d.part.as_deref().unwrap_or(main);
            reach(package, target(package, owner, relationship).as_deref(), &mut read);
        }
    }
    let slots: Vec<&DrawingSlot> = paint
        .drawings
        .iter()
        .chain(text_boxes.flat_map(|t| t.drawings.iter()))
        .collect();
    for slot in &slots {
        let part = slot.story.as_ref().map(|(_, part)| part.as_ref());
        let owner = version
            .drawings
            .as_ref()
            .and_then(|d| d.owner(part, &names.slot(slot)))
            .unwrap_or_else(|| main.to_owned());
        if let Some(graphic) = &slot.graphic {
            graphic_reads(package, &owner, graphic, &mut read);
        }
    }
    let effects = paint
        .lines
        .iter()
        .chain(paint.text_boxes.iter().flat_map(|t| t.lines.iter()))
        .any(|l| l.spans.iter().any(|s| s.effects.is_some()));
    if effects || !slots.is_empty() {
        let found: Vec<String> = package.relationships(main).map_or_else(
            |_| Vec::new(),
            |rels| {
                rels.iter()
                    .filter(|r| COLOURED_FROM.iter().any(|kind| rel_type_is(&r.rel_type, kind)))
                    .filter_map(|r| r.target_part.clone())
                    .collect()
            },
        );
        for part in found {
            reach(package, Some(&part), &mut read);
        }
    }
    let types = package.content_types().ok();
    read.into_iter()
        .map(|name| {
            let mime = types.as_ref().and_then(|t| t.of(&name)).map(str::to_owned);
            let content = memo.of(package, &name, key);
            (name, mime, content)
        })
        .collect()
}

/// The part `owner`'s relationship `id` targets.
fn target(package: &Package, owner: &str, id: &str) -> Option<String> {
    package.relationships(owner).ok()?.get(id)?.target_part.clone()
}

/// Part `name` and every part it relates to, through their relationships, into `read`.
fn reach(package: &Package, name: Option<&str>, read: &mut BTreeSet<String>) {
    let Some(name) = name.filter(|n| package.has_part(n)) else {
        return;
    };
    if !read.insert(name.to_owned()) {
        return;
    }
    let Ok(rels) = package.relationships(name) else {
        return;
    };
    let targets: Vec<String> = rels.iter().filter_map(|r| r.target_part.clone()).collect();
    for t in targets {
        reach(package, Some(&t), read);
    }
}

/// What a drawing's graphic reads from `owner`: its own relationship's part (a picture,
/// a chart, a diagram's data model), the parts its markup names by relationship id (a
/// shape's picture fill, a VML object's picture), and its members', each with what they
/// relate to; a diagram's drawings the owner relates to (`diagram::drawing`'s fallback).
fn graphic_reads(package: &Package, owner: &str, graphic: &Graphic, read: &mut BTreeSet<String>) {
    if let Some(relationship) = graphic.relationship.as_deref() {
        reach(package, target(package, owner, relationship).as_deref(), read);
    }
    if &*graphic.kind == "diagram" {
        if let Ok(rels) = package.relationships(owner) {
            let drawings: Vec<String> = rels
                .iter()
                .filter(|r| r.rel_type.ends_with("/diagramDrawing"))
                .filter_map(|r| r.target_part.clone())
                .collect();
            for d in drawings {
                reach(package, Some(&d), read);
            }
        }
    }
    if let Some(source) = &graphic.source {
        let mut ids = BTreeSet::new();
        named_ids(source, &mut ids);
        for id in ids {
            reach(package, target(package, owner, &id).as_deref(), read);
        }
    }
    for child in graphic.children.iter() {
        graphic_reads(package, owner, child, read);
    }
}

/// Every relationship id `element`'s markup names (`r:embed`, `r:id`, `r:link`...).
fn named_ids(element: &ooxml_xml::Element, out: &mut BTreeSet<String>) {
    for a in element.attributes() {
        if a.name.namespace().is_some_and(|ns| RELATIONSHIPS.contains(&ns)) {
            out.insert(a.value.to_string());
        }
    }
    for child in element.child_elements() {
        named_ids(child, out);
    }
}
