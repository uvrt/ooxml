//! Owns: the body's sections as docx-agent reads them (`edit/sections.py`): each ended by
//! a paragraph's `w:sectPr` (`s:<paraId>`, `s@body/<n>` for a paragraph without one) or the
//! body's last (`s:body`); their start, page, margins and columns as declared; and the
//! header and footer each shows, by kind, inherited from the sections before it; the
//! names of a header or footer ([`HeaderFooter`]) and of a section's start
//! ([`SectionBreakKind`]).
//!
//! Must not: change a section or lay out pages.

use crate::docx::{find_path, Docx};
use crate::ids::W;
use crate::xmlread::py_int;
use ooxml_xml::{Element, Node};
use std::sync::Arc;

const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The kinds of header and footer, in docx-agent's order.
pub const KINDS: [&str; 3] = ["default", "first", "even"];

/// A section's header or its footer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeaderOrFooter {
    /// `w:headerReference`, a `header*.xml` part.
    Header,
    /// `w:footerReference`, a `footer*.xml` part.
    Footer,
}

/// Which pages a header or footer shows on (`w:type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StoryKind {
    /// Every page the others do not take.
    Default,
    /// The section's first page.
    First,
    /// Even pages.
    Even,
}

/// One of a section's six headers and footers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HeaderFooter {
    /// Header or footer.
    pub part: HeaderOrFooter,
    /// Which pages.
    pub kind: StoryKind,
}

impl HeaderOrFooter {
    /// Its name (`header`, `footer`): the reference's and the part's stem.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            HeaderOrFooter::Header => "header",
            HeaderOrFooter::Footer => "footer",
        }
    }
}

impl StoryKind {
    /// Its `w:type` value (`default`, `first`, `even`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            StoryKind::Default => "default",
            StoryKind::First => "first",
            StoryKind::Even => "even",
        }
    }
}

/// How a section starts (`w:type` of `w:sectPr`, ST_SectionMark).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionBreakKind {
    /// On a new page.
    NextPage,
    /// On the same page.
    Continuous,
    /// On the next even page.
    EvenPage,
    /// On the next odd page.
    OddPage,
    /// In the next column.
    NextColumn,
}

impl SectionBreakKind {
    /// Its `w:type` value (`nextPage`...).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            SectionBreakKind::NextPage => "nextPage",
            SectionBreakKind::Continuous => "continuous",
            SectionBreakKind::EvenPage => "evenPage",
            SectionBreakKind::OddPage => "oddPage",
            SectionBreakKind::NextColumn => "nextColumn",
        }
    }
}

/// A section's text columns as it states them (docx-agent's `Section.columns`).
#[derive(Debug, Clone, PartialEq)]
pub struct Columns {
    /// How many.
    pub count: i64,
    /// The space between them, points (36 where it states none; `None` for a bad value).
    pub space: Option<f64>,
    /// Whether a line is drawn between them.
    pub separator: bool,
    /// For unequal columns, each one's width and the space after it, points.
    pub widths: Vec<(Option<f64>, f64)>,
}

/// The margins docx-agent reads, in its order.
pub const MARGINS: [&str; 7] = ["top", "right", "bottom", "left", "header", "footer", "gutter"];

/// A section.
#[derive(Debug, Clone)]
pub struct Section {
    /// Its id.
    pub id: String,
    /// Its `w:sectPr`.
    pub properties: Arc<Element>,
    /// The `w:sectPr`'s place in the main part.
    pub path: Box<[u32]>,
}

fn int_of(node: Option<&Element>, local: &str, default: Option<i64>) -> Option<i64> {
    match node {
        None => default,
        Some(n) => n.attribute(Some(W), local).and_then(py_int).or(default),
    }
}

/// Twips as points (`int(value) / 20`), `None` where absent or not a number.
#[must_use]
pub fn points(value: Option<&str>) -> Option<f64> {
    value.and_then(py_int).map(|v| v as f64 / 20.0)
}

impl Section {
    /// How it starts (`nextPage` where it states none).
    #[must_use]
    pub fn start(&self) -> String {
        self.properties
            .find(Some(W), "type")
            .and_then(|t| t.attribute(Some(W), "val"))
            .filter(|v| !v.is_empty())
            .unwrap_or("nextPage")
            .to_owned()
    }

    /// `portrait` or `landscape`.
    #[must_use]
    pub fn orientation(&self) -> &'static str {
        let size = self.properties.find(Some(W), "pgSz");
        if size.and_then(|s| s.attribute(Some(W), "orient")) == Some("landscape") {
            "landscape"
        } else {
            "portrait"
        }
    }

    /// The page's width and height in points, where it states a size.
    #[must_use]
    pub fn page(&self) -> Option<(f64, f64)> {
        let size = self.properties.find(Some(W), "pgSz")?;
        let get = |local| int_of(Some(size), local, Some(0)).unwrap_or(0) as f64 / 20.0;
        Some((get("w"), get("h")))
    }

    /// A margin in points (`top`, `right`, `bottom`, `left`...).
    #[must_use]
    pub fn margin(&self, side: &str) -> Option<f64> {
        points(self.properties.find(Some(W), "pgMar")?.attribute(Some(W), side))
    }

    /// The number of columns.
    #[must_use]
    pub fn columns(&self) -> i64 {
        let node = self.properties.find(Some(W), "cols").map(|n| &**n);
        let widths = node.map_or(0, |n| {
            n.child_elements().filter(|c| c.name().is(Some(W), "col")).count()
        });
        if widths > 0 {
            return i64::try_from(widths).unwrap_or(i64::MAX);
        }
        match int_of(node, "num", Some(1)) {
            Some(0) | None => 1,
            Some(n) => n,
        }
    }

    /// The width between the margins and gutter, twips, where it states a page size.
    #[must_use]
    pub fn text_width(&self) -> Option<i64> {
        let size = self.properties.find(Some(W), "pgSz")?;
        let width = int_of(Some(size), "w", Some(0)).unwrap_or(0);
        let margins = self.properties.find(Some(W), "pgMar").map(|m| &**m);
        let side = |local| int_of(margins, local, Some(0)).unwrap_or(0);
        Some(width - side("left") - side("right") - side("gutter"))
    }

    /// Its text columns.
    #[must_use]
    pub fn column_layout(&self) -> Columns {
        let node = self.properties.find(Some(W), "cols").map(|n| &**n);
        let space = node
            .and_then(|n| n.attribute(Some(W), "space"))
            .filter(|s| !s.is_empty());
        let separator = node
            .and_then(|n| n.attribute(Some(W), "sep"))
            .filter(|s| !s.is_empty())
            .is_some_and(super::on);
        let widths = node.map_or_else(Vec::new, |n| {
            n.child_elements()
                .filter(|c| c.name().is(Some(W), "col"))
                .map(|c| {
                    let width = points(c.attribute(Some(W), "w"));
                    let after = points(c.attribute(Some(W), "space")).filter(|v| *v != 0.0);
                    (width, after.unwrap_or(0.0))
                })
                .collect()
        });
        Columns {
            count: self.columns(),
            space: match space {
                Some(s) => points(Some(s)),
                None => Some(36.0),
            },
            separator,
            widths,
        }
    }

    /// `w:titlePg`: whether the first page shows the `first` header and footer.
    #[must_use]
    pub fn title_page(&self) -> bool {
        self.properties.find(Some(W), "titlePg").is_some_and(|n| {
            let value = n.attribute(Some(W), "val").filter(|v| !v.is_empty()).unwrap_or("true");
            !matches!(value.to_lowercase().as_str(), "0" | "false" | "off")
        })
    }

    /// Whether it states its own `which` (`header`, `footer`) of `kind`.
    #[must_use]
    pub fn owns(&self, which: &str, kind: &str) -> bool {
        self.own_reference(which, kind).is_some()
    }

    fn own_reference(&self, which: &str, kind: &str) -> Option<&Arc<Element>> {
        let local = if which == "header" {
            "headerReference"
        } else {
            "footerReference"
        };
        self.properties
            .child_elements()
            .filter(|n| n.name().is(Some(W), local))
            .find(|n| n.attribute(Some(W), "type").unwrap_or("default") == kind)
    }
}

/// Every section of the body, in order.
#[must_use]
pub fn sections(docx: &Docx) -> Vec<Section> {
    let Some(body) = docx.body() else { return Vec::new() };
    let mut out = Vec::new();
    let mut ancestors: Vec<(Arc<Element>, Box<[u32]>)> = Vec::new();
    let mut path = Vec::new();
    find_breaks(docx, body.root(), &mut ancestors, &mut path, &mut out);
    let root = body.root();
    let at = |e: &Element, local: &str| {
        e.children()
            .iter()
            .position(|n| matches!(n, Node::Element(c) if c.name().is(Some(W), local)))
    };
    if let Some(b) = at(root, "body") {
        if let Some(Node::Element(body_element)) = root.children().get(b) {
            if let Some(s) = at(body_element, "sectPr") {
                if let Some(Node::Element(last)) = body_element.children().get(s) {
                    out.push(Section {
                        id: "s:body".to_owned(),
                        properties: Arc::clone(last),
                        path: [b, s].map(|k| u32::try_from(k).unwrap_or(u32::MAX)).into(),
                    });
                }
            }
        }
    }
    out
}

fn find_breaks(
    docx: &Docx,
    element: &Arc<Element>,
    ancestors: &mut Vec<(Arc<Element>, Box<[u32]>)>,
    path: &mut Vec<u32>,
    out: &mut Vec<Section>,
) {
    for (k, node) in element.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        if child.name().is(Some(W), "sectPr") {
            if let Some(id) = break_id(docx, element, ancestors, k) {
                out.push(Section {
                    id,
                    properties: Arc::clone(child),
                    path: path.clone().into_boxed_slice(),
                });
            }
        }
        if holds_breaks(element, child) {
            ancestors.push((Arc::clone(element), path.clone().into_boxed_slice()));
            find_breaks(docx, child, ancestors, path, out);
            ancestors.pop();
        }
        path.pop();
    }
}

/// Whether `child` (of `parent`) can hold a section break ([`break_id`]): the body, the
/// containers between it and its paragraphs, a paragraph, and a paragraph's properties.
/// Anything else (a run, a table, a cell's paragraphs) holds none, so the walk does not go
/// into it.
fn holds_breaks(parent: &Element, child: &Element) -> bool {
    let name = child.name();
    if name.is(Some(W), "pPr") {
        return parent.name().is(Some(W), "p");
    }
    ["body", "p", "sdt", "sdtContent", "customXml"]
        .iter()
        .any(|local| name.is(Some(W), local))
        && !parent.name().is(Some(W), "p")
}

/// The section id of a `w:sectPr` in `properties`, where it ends a section: the first
/// `w:sectPr` of a paragraph's first `w:pPr`, the paragraph a block of the body.
fn break_id(
    docx: &Docx,
    properties: &Arc<Element>,
    ancestors: &[(Arc<Element>, Box<[u32]>)],
    sect: usize,
) -> Option<String> {
    if !properties.name().is(Some(W), "pPr") {
        return None;
    }
    // Each ancestor is held with its child's place: the last is (paragraph, place of properties).
    let (paragraph, properties_path) = ancestors.last()?;
    let paragraph_path = properties_path.get(..properties_path.len().checked_sub(1)?)?;
    let own = *properties_path.last()? as usize;
    if !paragraph.name().is(Some(W), "p")
        || first_index(paragraph, "pPr") != Some(own)
        || first_index(properties, "sectPr") != Some(sect)
    {
        return None;
    }
    let containers = ancestors.get(..ancestors.len().checked_sub(1)?)?;
    let in_body = containers.iter().rev().map(|(e, _)| e).find(|e| {
        !(e.name().is(Some(W), "sdtContent") || e.name().is(Some(W), "sdt") || e.name().is(Some(W), "customXml"))
    });
    if !in_body.is_some_and(|e| e.name().is(Some(W), "body")) {
        return None;
    }
    let body = docx.body()?;
    let entry = body.ids.paragraph_at(paragraph_path)?;
    Some(section_id(&entry.id))
}

/// The index among its parent's children of `element`'s first child `w:{local}`.
fn first_index(element: &Element, local: &str) -> Option<usize> {
    element
        .children()
        .iter()
        .position(|n| matches!(n, Node::Element(e) if e.name().is(Some(W), local)))
}

/// A section's id from the id of the paragraph ending it.
#[must_use]
pub fn section_id(paragraph_id: &str) -> String {
    format!("s:{}", paragraph_id.rsplit("p:").next().unwrap_or(paragraph_id))
}

/// The story section `index` of `sections` shows for `which` (`header`, `footer`) and
/// `kind`: its own reference, else the last a section before it states.
#[must_use]
pub fn section_story(docx: &Docx, sections: &[Section], index: usize, which: &str, kind: &str) -> Option<String> {
    let node = (0..=index)
        .rev()
        .find_map(|k| sections.get(k).and_then(|s| s.own_reference(which, kind)))?;
    let rid = node.attribute(Some(R), "id")?;
    let rels = docx.relationships(&docx.main).ok()?;
    let part = rels.get(rid)?.target_part.clone()?;
    docx.part(&part).map(|p| p.story.clone())
}

/// Where a paragraph's `w:pPr/w:sectPr` is.
#[must_use]
pub fn ends_section(paragraph: &Element) -> bool {
    find_path(paragraph, &["pPr", "sectPr"]).is_some()
}

#[cfg(test)]
mod tests {
    //! The section breaks found walking only where one can stand equal those a walk of the
    //! whole body finds, on every docx-agent fixture.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::{break_id, sections, Section, W};
    use crate::docx::Docx;
    use ooxml_xml::{Element, Node};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    /// Every element walked, as the breaks were found before lane L4a.
    fn every_break(
        docx: &Docx,
        element: &Arc<Element>,
        ancestors: &mut Vec<(Arc<Element>, Box<[u32]>)>,
        path: &mut Vec<u32>,
        out: &mut Vec<(String, Box<[u32]>)>,
    ) {
        for (k, node) in element.children().iter().enumerate() {
            let Node::Element(child) = node else { continue };
            path.push(u32::try_from(k).unwrap());
            if child.name().is(Some(W), "sectPr") {
                if let Some(id) = break_id(docx, element, ancestors, k) {
                    out.push((id, path.clone().into_boxed_slice()));
                }
            }
            ancestors.push((Arc::clone(element), path.clone().into_boxed_slice()));
            every_break(docx, child, ancestors, path, out);
            ancestors.pop();
            path.pop();
        }
    }

    fn fixtures(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                fixtures(&path, out);
            } else if path.extension().is_some_and(|e| e == "docx") {
                out.push(path);
            }
        }
    }

    #[test]
    fn breaks_found_where_they_can_stand_equal_a_whole_walk() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/ooxml-oracle/fixtures");
        let mut paths = Vec::new();
        fixtures(&dir, &mut paths);
        let mut breaks = 0;
        for path in paths {
            let bytes: Arc<[u8]> = Arc::from(std::fs::read(&path).unwrap());
            let Ok(package) = ooxml_opc::Package::open(bytes, &ooxml_base::Limits::default()) else {
                continue;
            };
            let Ok(docx) = Docx::open(
                &crate::WordPackage::open(package).0,
                std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX)),
            ) else {
                continue;
            };
            let Some(body) = docx.body() else { continue };
            let mut whole = Vec::new();
            every_break(&docx, body.root(), &mut Vec::new(), &mut Vec::new(), &mut whole);
            let found: Vec<(String, Box<[u32]>)> = sections(&docx)
                .into_iter()
                .filter(|s| s.id != "s:body")
                .map(|Section { id, path, .. }| (id, path))
                .collect();
            assert_eq!(found, whole, "{}", path.display());
            breaks += found.len();
        }
        assert!(breaks > 1, "{breaks} breaks");
    }
}
