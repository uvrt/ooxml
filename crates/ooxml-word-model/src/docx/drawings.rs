//! Owns: drawings as docx-agent names them (`edit/pictures.py`, `edit/annotations.py`,
//! `edit/charts.py`): a picture's `d:<docPr id>` (`#<k>` for a repeat among pictures),
//! every other drawing's `d:<docPr id>` (`#<k>` among all drawings), a group's members
//! `d:<group>/<cNvPr id>`, depth first; and what kind each is.
//!
//! Must not: change a drawing or read its geometry.

use crate::docx::{live_paths, Docx};
use crate::ids::{W, WP};
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// DrawingML's main namespace.
pub const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// DrawingML pictures.
pub const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
/// Word 2010's groups.
pub const WPG: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup";
/// Word 2010's shapes.
pub const WPS: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape";
/// DrawingML charts.
pub const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
/// DrawingML diagrams.
pub const DGM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";

/// A drawing or group member.
#[derive(Debug, Clone)]
pub struct Drawing {
    /// Its part.
    pub part: String,
    /// Its id.
    pub id: String,
    /// The `wp:inline` or `wp:anchor` (a member: its own element).
    pub element: Arc<Element>,
    /// What it is: `picture`, `chart`, `diagram`, `text-box`, `group`, `canvas`, `shape`,
    /// `other`.
    pub kind: &'static str,
    /// Whether it is a group's member.
    pub member: bool,
    /// Its frame's place in its part (a member: its group's).
    pub path: Box<[u32]>,
}

impl Drawing {
    /// Its alternative text: a drawing's `wp:docPr/@descr`, a member's `cNvPr/@descr`.
    #[must_use]
    pub fn alt_text(&self) -> Option<&str> {
        if self.member {
            member_properties(&self.element)?.attribute(None, "descr")
        } else {
            self.element.find(Some(WP), "docPr")?.attribute(None, "descr")
        }
    }
}

fn frames(docx: &Docx) -> Vec<(String, Arc<Element>, Box<[u32]>)> {
    let mut out = Vec::new();
    for part in &docx.parts {
        let found = live_paths(part.root(), &[], &|e| {
            e.name().is(Some(WP), "inline") || e.name().is(Some(WP), "anchor")
        });
        out.extend(found.into_iter().map(|f| (part.part.clone(), f.element, f.path)));
    }
    out
}

fn doc_pr_id(frame: &Element) -> Option<&str> {
    frame.find(Some(WP), "docPr")?.attribute(None, "id")
}

fn numbered(seen: &mut HashMap<String, usize>, raw: &str) -> String {
    let count = seen.entry(raw.to_owned()).or_insert(0);
    let id = if *count > 0 {
        format!("d:{raw}#{count}")
    } else {
        format!("d:{raw}")
    };
    *count += 1;
    id
}

/// Every picture (a frame holding a `pic:pic`), in story and document order.
#[must_use]
pub fn pictures(docx: &Docx) -> Vec<Drawing> {
    let mut seen = HashMap::new();
    frames(docx)
        .into_iter()
        .filter(|(_, frame, _)| has_descendant(frame, PIC, "pic"))
        .filter_map(|(part, frame, path)| {
            let id = numbered(&mut seen, doc_pr_id(&frame)?);
            Some(Drawing {
                part,
                id,
                element: frame,
                kind: "picture",
                member: false,
                path,
            })
        })
        .collect()
}

/// Every drawing with a `docPr` id, a picture under its picture id.
#[must_use]
pub fn all_drawings(docx: &Docx) -> Vec<Drawing> {
    let mut seen = HashMap::new();
    let mut seen_pictures = HashMap::new();
    let mut out = Vec::new();
    for (part, frame, path) in frames(docx) {
        let Some(raw) = doc_pr_id(&frame) else { continue };
        let id = numbered(&mut seen, raw);
        let id = if has_descendant(&frame, PIC, "pic") {
            numbered(&mut seen_pictures, raw)
        } else {
            id
        };
        out.push(Drawing {
            part,
            id,
            kind: drawing_kind(&frame),
            element: frame,
            member: false,
            path,
        });
    }
    out
}

/// Every drawing and group member, in order (docx-agent's `_graphics`).
#[must_use]
pub fn graphics(docx: &Docx) -> Vec<Drawing> {
    let mut out = Vec::new();
    for drawing in all_drawings(docx) {
        let group = drawing.kind == "group";
        let (part, id, element, path) = (
            drawing.part.clone(),
            drawing.id.clone(),
            Arc::clone(&drawing.element),
            drawing.path.clone(),
        );
        out.push(drawing);
        if group {
            for (member, node) in group_members(&element, &id, false) {
                let kind = member_kind(&node);
                out.push(Drawing {
                    part: part.clone(),
                    id: member,
                    kind,
                    element: node,
                    member: true,
                    path: path.clone(),
                });
            }
        }
    }
    out
}

/// What a frame holds, from its `a:graphicData`.
#[must_use]
pub fn drawing_kind(frame: &Element) -> &'static str {
    let data = frame
        .find(Some(A), "graphic")
        .and_then(|g| g.find(Some(A), "graphicData"));
    let uri = data.and_then(|d| d.attribute(None, "uri")).unwrap_or("");
    if data.is_some_and(|d| d.find(Some(PIC), "pic").is_some()) {
        return "picture";
    }
    if uri.ends_with("/chart") {
        return "chart";
    }
    if uri.ends_with("/diagram") {
        return "diagram";
    }
    if has_descendant(frame, W, "txbxContent") {
        return "text-box";
    }
    if uri.ends_with("wordprocessingGroup") {
        return "group";
    }
    if uri.ends_with("wordprocessingCanvas") {
        return "canvas";
    }
    if uri.ends_with("wordprocessingShape") {
        return "shape";
    }
    "other"
}

/// A group member's kind.
#[must_use]
pub fn member_kind(node: &Element) -> &'static str {
    if node.name().is(Some(WPG), "grpSp") {
        return "group";
    }
    if let Some(data) = node
        .find(Some(A), "graphic")
        .and_then(|g| g.find(Some(A), "graphicData"))
    {
        if data.find(Some(C), "chart").is_some() {
            return "chart";
        }
        if data.find(Some(DGM), "relIds").is_some() {
            return "diagram";
        }
        return "other";
    }
    if node.name().is(Some(WPS), "wsp") || node.name().is(Some(WPG), "wsp") {
        return if node.find(Some(WPS), "txbx").is_some() {
            "text-box"
        } else {
            "shape"
        };
    }
    if node.name().local() == "pic" && node.name().namespace().is_some() {
        return "picture";
    }
    "other"
}

fn member_properties(node: &Element) -> Option<&Element> {
    node.find(Some(WPG), "cNvPr")
        .or_else(|| node.find(Some(WPS), "cNvPr"))
        .map(|e| &**e)
        .or_else(|| node.find(Some(PIC), "nvPicPr")?.find(Some(PIC), "cNvPr").map(|e| &**e))
}

/// Every member of a group drawing (or a nested `wpg:grpSp` when `nested`), depth first.
pub fn group_members(group: &Element, prefix: &str, nested: bool) -> Vec<(String, Arc<Element>)> {
    let holder = if nested {
        Some(group)
    } else {
        group
            .find(Some(A), "graphic")
            .and_then(|g| g.find(Some(A), "graphicData"))
            .and_then(|d| d.find(Some(WPG), "wgp"))
            .map(|e| &**e)
    };
    let Some(holder) = holder else { return Vec::new() };
    let mut out = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for child in holder.child_elements() {
        let name = child.name();
        let member = name.is(Some(WPG), "graphicFrame")
            || name.is(Some(WPG), "grpSp")
            || name.is(Some(WPS), "wsp")
            || name.is(Some(WPG), "wsp")
            || (name.local() == "pic" && name.namespace().is_some());
        if !member {
            continue;
        }
        let Some(raw) = member_properties(child).and_then(|p| p.attribute(None, "id")) else {
            continue;
        };
        let count = seen.entry(raw.to_owned()).or_insert(0);
        let id = if *count > 0 {
            format!("{prefix}/{raw}#{count}")
        } else {
            format!("{prefix}/{raw}")
        };
        *count += 1;
        out.push((id.clone(), Arc::clone(child)));
        if name.is(Some(WPG), "grpSp") {
            out.extend(group_members(child, &id, true));
        }
    }
    out
}

/// Whether an element below `element` is `{namespace}local` (fallbacks included).
#[must_use]
pub fn has_descendant(element: &Element, namespace: &str, local: &str) -> bool {
    let mut stack = vec![element.children().iter()];
    while let Some(top) = stack.last_mut() {
        match top.next() {
            Some(Node::Element(e)) => {
                if e.name().is(Some(namespace), local) {
                    return true;
                }
                stack.push(e.children().iter());
            }
            Some(_) => {}
            None => {
                stack.pop();
            }
        }
    }
    false
}
