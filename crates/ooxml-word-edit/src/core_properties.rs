//! Owns: the package's core properties (`docProps/core.xml`) as docx-agent's
//! `edit/properties.py` reads and writes them: [`core_properties`], each property the part
//! sets as its text; [`SetCoreProperties`], properties set in place -- a new element where
//! Word writes it (after the last one before it in [`CoreProperty`]'s order, else first), the
//! part made with its relationship from the package and its content type when there is none
//! (`core_part(create=True)`), nothing written when every value is already the one given.
//!
//! Must not: write the dates (`dcterms:created`, `dcterms:modified`) or the revision -- the
//! tools set text properties only -- or touch any part but the core part, the package's
//! relationships and the content types.

use crate::doc::{Key, WordDoc};
use crate::new_part::add_part;
use crate::policy::RevisionPolicy;
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_opc::name::rels_for;
use ooxml_opc::{Package, CONTENT_TYPES_PART};
use ooxml_store::{Command, Txn};
use ooxml_word_model::parts::{related_parts, REL_CORE};
use ooxml_xml::{Element, NamespaceDecl, Node, QName};
use std::sync::Arc;

const CP: &str = "http://schemas.openxmlformats.org/package/2006/metadata/core-properties";
const DC: &str = "http://purl.org/dc/elements/1.1/";
const DCTERMS: &str = "http://purl.org/dc/terms/";
const DCMITYPE: &str = "http://purl.org/dc/dcmitype/";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
/// A date's `xsi:type`.
const W3CDTF: &str = "dcterms:W3CDTF";
/// The core part's content type.
const CORE_TYPE: &str = "application/vnd.openxmlformats-package.core-properties+xml";
/// Where a new core part goes.
const CORE_PART: &str = "docProps/core.xml";

/// A core property, in the order Word writes them (docx-agent's `CORE_PROPERTIES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoreProperty {
    /// `dc:title`.
    Title,
    /// `dc:subject`.
    Subject,
    /// `dc:creator`.
    Author,
    /// `cp:keywords`.
    Keywords,
    /// `dc:description`.
    Description,
    /// `cp:lastModifiedBy`.
    LastModifiedBy,
    /// `cp:revision`.
    Revision,
    /// `dcterms:created`.
    Created,
    /// `dcterms:modified`.
    Modified,
    /// `cp:category`.
    Category,
    /// `cp:contentStatus`.
    Status,
    /// `dc:language`.
    Language,
}

impl CoreProperty {
    /// Every property, in Word's order.
    pub const ALL: [CoreProperty; 12] = [
        Self::Title,
        Self::Subject,
        Self::Author,
        Self::Keywords,
        Self::Description,
        Self::LastModifiedBy,
        Self::Revision,
        Self::Created,
        Self::Modified,
        Self::Category,
        Self::Status,
        Self::Language,
    ];

    /// The element: its namespace, prefix and local name.
    #[must_use]
    pub fn element(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Title => (DC, "dc", "title"),
            Self::Subject => (DC, "dc", "subject"),
            Self::Author => (DC, "dc", "creator"),
            Self::Keywords => (CP, "cp", "keywords"),
            Self::Description => (DC, "dc", "description"),
            Self::LastModifiedBy => (CP, "cp", "lastModifiedBy"),
            Self::Revision => (CP, "cp", "revision"),
            Self::Created => (DCTERMS, "dcterms", "created"),
            Self::Modified => (DCTERMS, "dcterms", "modified"),
            Self::Category => (CP, "cp", "category"),
            Self::Status => (CP, "cp", "contentStatus"),
            Self::Language => (DC, "dc", "language"),
        }
    }

    /// The property an element of this name holds.
    fn named(name: &QName) -> Option<CoreProperty> {
        Self::ALL.into_iter().find(|p| {
            let (ns, _, local) = p.element();
            name.is(Some(ns), local)
        })
    }
}

/// The core part: the first target of the package's core-properties relationship that is
/// a part (docx-agent's `_part`).
///
/// # Errors
/// When the package's relationships are not XML.
pub fn core_part(package: &Package) -> Result<Option<String>> {
    let found = related_parts(package, "", REL_CORE).map_err(crate::doc::ModelError::from)?;
    Ok(found.into_iter().find(|p| package.has_part(p)))
}

/// Each property the core part sets, with its text (`""` for an empty element), in
/// [`CoreProperty`]'s order; nothing without a core part.
///
/// # Errors
/// When the core part is not XML.
pub fn core_properties(doc: &WordDoc) -> Result<Vec<(CoreProperty, String)>> {
    let Some(part) = core_part(doc.package())? else {
        return Ok(Vec::new());
    };
    let Some(tree) = doc.tree(&part)? else {
        return Ok(Vec::new());
    };
    let root = tree.root();
    Ok(CoreProperty::ALL
        .into_iter()
        .filter_map(|p| {
            let (ns, _, local) = p.element();
            // an element's text as lxml's `.text`: the text before its first child
            root.find(Some(ns), local)
                .map(|e| (p, e.children().get(0).and_then(Node::as_text).unwrap_or("").to_owned()))
        })
        .collect())
}

/// Set text properties: each to its value, one step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetCoreProperties {
    /// The properties and their new text, in the order given.
    pub values: Vec<(CoreProperty, String)>,
}

/// What setting the properties did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreSet {
    /// Whether it changed the document.
    pub changed: bool,
}

/// `<cp:coreProperties>` with docx-agent's five declarations, empty.
fn empty_core() -> Element {
    let ns = |prefix: &str, uri: &str| NamespaceDecl {
        prefix: Some(Arc::from(prefix)),
        uri: Arc::from(uri),
    };
    Element::new(
        Arc::new(QName::new(Some("cp"), "coreProperties", Some(CP))),
        vec![
            ns("cp", CP),
            ns("dc", DC),
            ns("dcterms", DCTERMS),
            ns("dcmitype", DCMITYPE),
            ns("xsi", XSI),
        ],
        Vec::new(),
        Vec::new(),
    )
}

impl SetCoreProperties {
    /// Whether every value is already the document's.
    fn already(&self, doc: &WordDoc) -> Result<bool> {
        let current = core_properties(doc)?;
        Ok(self
            .values
            .iter()
            .all(|(p, v)| current.iter().any(|(q, text)| q == p && text == v)))
    }
}

impl Command<WordDoc> for SetCoreProperties {
    type Out = CoreSet;
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<()> {
        if self
            .values
            .iter()
            .any(|(p, _)| matches!(p, CoreProperty::Created | CoreProperty::Modified))
        {
            return Err(EditError::Refused("the dates are not set as text".to_owned()));
        }
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<CoreSet> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        for part in [CORE_PART, CONTENT_TYPES_PART, rels_for("").as_str()] {
            sink.read(Key::part(part));
        }
        let found = core_part(sink.doc().package())?;
        if let Some(part) = &found {
            sink.read(Key::part(part.as_str()));
        }
        if self.already(sink.doc())? {
            return Ok(CoreSet { changed: false });
        }
        let changed = write_core(
            &mut sink,
            &self
                .values
                .iter()
                .map(|(p, v)| (*p, Some(v.as_str())))
                .collect::<Vec<_>>(),
        )?;
        Ok(CoreSet { changed })
    }
}

/// Set each property to its value (`None`: removed), in the order given -- a new element
/// where Word writes it (after the last one before it in [`CoreProperty`]'s order, else
/// first), a date marked `xsi:type="dcterms:W3CDTF"` -- the core part made when there is
/// none (docx-agent's `write_core`). Whether anything changed.
///
/// # Errors
/// When the core part, the content types or the package's relationships are not XML.
pub(crate) fn write_core(sink: &mut EditSink<'_, '_>, values: &[(CoreProperty, Option<&str>)]) -> Result<bool> {
    for part in [CORE_PART, CONTENT_TYPES_PART, rels_for("").as_str()] {
        sink.read(Key::part(part));
    }
    let found = core_part(sink.doc().package())?;
    if let Some(part) = &found {
        sink.read(Key::part(part.as_str()));
    }
    let part = match found {
        Some(part) => part,
        None => {
            // docx-agent names it `unused_part_name("docProps/core.xml")` when that name
            // is taken, which never ends; numbered here instead
            let package = sink.doc().package().clone();
            let part = crate::new_part::unused_name(&package, CORE_PART, "docProps/core{n}.xml");
            let made = add_part(&package, "", &part, empty_core(), CORE_TYPE, REL_CORE)?;
            for name in [part.as_str(), CONTENT_TYPES_PART, rels_for("").as_str()] {
                if let Some(tree) = made.tree(name).map_err(crate::doc::ModelError::from)? {
                    sink.put(name, tree)?;
                }
            }
            part
        }
    };
    let held = sink.doc().clone();
    let mut space = Space::default();
    let work = space
        .work(&held, &part)?
        .ok_or_else(|| EditError::no_part(part.clone()))?;
    let root = work.root();
    let mut changed = false;
    for (property, value) in values {
        let (ns, _, local) = property.element();
        let found = work.find(root, ns, local);
        let Some(value) = value else {
            if let Some(node) = found {
                work.remove(node);
                changed = true;
            }
            continue;
        };
        let node = match found {
            Some(node) => node,
            None => {
                let declared = work.has_prefix_for(root, ns);
                // lxml names a namespace nothing declares `ns0`, declared on the element
                let node = work.new_element(work.element_name(root, ns, local, "ns0"));
                if !declared {
                    work.declare(node, "ns0", ns);
                }
                let after = work.children(root).iter().copied().rfind(|c| {
                    work.name(*c)
                        .and_then(|n| CoreProperty::named(n))
                        .is_some_and(|q| q < *property)
                });
                match after {
                    Some(after) => work.add_next(after, node),
                    None => work.insert(root, 0, node),
                }
                changed = true;
                node
            }
        };
        if matches!(property, CoreProperty::Created | CoreProperty::Modified)
            && work.get(node, Some(XSI), "type") != Some(W3CDTF)
        {
            work.set(node, work.qname(XSI, "type", "xsi"), W3CDTF);
            changed = true;
        }
        if work.text(node) != *value {
            work.set_text(node, value);
            changed = true;
        }
    }
    if changed {
        space.mark_dirty(&part);
    }
    space.finish(sink)?;
    Ok(changed)
}
