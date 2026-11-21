//! Owns: a SmartArt diagram's data model (`dgm:dataModel`, the diagram's truth) read as an
//! editor shows it, as ooxml-edit's `DiagramModel.walk` and `diagram_model` read it: the
//! nodes (points of type `node` or `asst`; a point without a type is a node), depth first
//! from the document point, each under its parent by a `parOf` connection (or one without a
//! type) in `srcOrd` order, each node once; with its level and its text (its `dgm:t`'s
//! paragraphs, one per line); and the one-line summary docx-agent and pptx-agent write of it
//! (`Goals [Faster edits, Fewer prompts], Risks`).
//!
//! The cached drawing (`dsp:drawing`), which the painters read, is not read here: the data
//! model is what an application lays the diagram out from.
//!
//! Must not: lay a diagram out, or change one.

use crate::read::text::paragraph_text;
use ooxml_xml::view::{attr, child, children, py_flat as flat, py_int};
use ooxml_xml::Element;
use std::collections::{HashMap, HashSet};

/// How deep a diagram's nodes may nest before it reads as unreadable: docx-agent's walk
/// recurses, and Python stops a recursion near a thousand frames.
pub const MAX_DEPTH: usize = 900;

/// One node of a diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramNode {
    /// Its `modelId`.
    pub id: Option<String>,
    /// Its depth: 0 for a top-level node.
    pub level: usize,
    /// Its text, paragraphs joined by `\n`, a line break as `\v`.
    pub text: String,
}

/// A diagram nested deeper than [`MAX_DEPTH`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooDeep;

/// The nodes of the data model `root`, depth first in order.
///
/// # Errors
/// [`TooDeep`] where nodes nest deeper than [`MAX_DEPTH`].
pub fn diagram_nodes(root: &Element) -> Result<Vec<DiagramNode>, TooDeep> {
    let points: Vec<&Element> = child(root, "ptLst")
        .into_iter()
        .flat_map(|l| children(l, "pt"))
        .collect();
    // a dictionary by modelId: each id at its first place, holding its last point
    let mut by_id: HashMap<Option<&str>, &Element> = HashMap::new();
    let mut order: Vec<Option<&str>> = Vec::new();
    for point in &points {
        let id = attr(point, "modelId");
        if by_id.insert(id, point).is_none() {
            order.push(id);
        }
    }
    let Some(document) = order
        .iter()
        .filter_map(|id| by_id.get(id))
        .find(|p| attr(p, "type") == Some("doc"))
    else {
        return Ok(Vec::new());
    };
    let connections: Vec<&Element> = child(root, "cxnLst")
        .into_iter()
        .flat_map(|l| children(l, "cxn"))
        .collect();
    let children_of = |parent: Option<&str>| -> Vec<&Element> {
        let mut found: Vec<(i64, &Element)> = connections
            .iter()
            .filter(|c| matches!(attr(c, "type"), None | Some("parOf")) && attr(c, "srcId") == parent)
            .filter_map(|c| {
                let point = by_id.get(&attr(c, "destId"))?;
                matches!(
                    attr(point, "type").filter(|t| !t.is_empty()).unwrap_or("node"),
                    "node" | "asst"
                )
                .then(|| (attr(c, "srcOrd").map_or(Some(0), py_int).unwrap_or(0), *point))
            })
            .collect();
        found.sort_by_key(|(order, _)| *order);
        found.into_iter().map(|(_, p)| p).collect()
    };
    let mut out = Vec::new();
    let mut seen: HashSet<Option<&str>> = HashSet::new();
    let mut stack = vec![children_of(attr(document, "modelId")).into_iter()];
    while let Some(top) = stack.last_mut() {
        let Some(point) = top.next() else {
            stack.pop();
            continue;
        };
        let id = attr(point, "modelId");
        if !seen.insert(id) {
            continue;
        }
        let level = stack.len() - 1;
        if level >= MAX_DEPTH {
            return Err(TooDeep);
        }
        out.push(DiagramNode {
            id: id.map(str::to_owned),
            level,
            text: node_text(point),
        });
        stack.push(children_of(id).into_iter());
    }
    Ok(out)
}

/// A point's text: its `dgm:t`'s paragraphs, one per line; empty without one.
fn node_text(point: &Element) -> String {
    child(point, "t").map_or_else(String::new, |t| {
        children(t, "p").map(paragraph_text).collect::<Vec<_>>().join("\n")
    })
}

/// The nodes' text, depth first, a node's children in brackets after it: `Goals [Faster
/// edits, Fewer prompts], Risks [Stale caches]` (docx-agent's `diagram_summary`).
#[must_use]
pub fn diagram_summary(nodes: &[DiagramNode]) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for (k, node) in nodes.iter().enumerate() {
        let level = if k == 0 { 0 } else { node.level.min(depth + 1) };
        if level > depth {
            out.push_str(" [");
        } else {
            out.push_str(&"]".repeat(depth - level));
            if k > 0 {
                out.push_str(", ");
            }
        }
        out.push_str(&flat(&node.text));
        depth = level;
    }
    out + &"]".repeat(depth)
}

#[cfg(test)]
mod tests {
    use super::{diagram_nodes, diagram_summary, DiagramNode};

    /// What ooxml-edit's `diagram_model` and docx-agent's `diagram_summary` give for the same
    /// part (worked out with them, 2026-11-20): connections in `srcOrd` order (one not a number
    /// is 0), a point without a type and an assistant are nodes, a presentation point and a
    /// transition are not, a cycle is walked once, paragraphs and line breaks.
    const MODEL: &str = r#"<dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dgm:ptLst><dgm:pt modelId="0" type="doc"><dgm:t><a:p><a:r><a:t>doc</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="1"><dgm:t><a:bodyPr/><a:p><a:r><a:t>Goals</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="2" type="node"><dgm:t><a:p><a:r><a:t>Faster</a:t></a:r><a:br/><a:r><a:t>edits</a:t></a:r></a:p><a:p><a:r><a:t>now</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="3" type="asst"><dgm:t><a:p><a:r><a:t>Helper</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="4" type="pres"/><dgm:pt modelId="5"/><dgm:pt modelId="6" type="parTrans"/></dgm:ptLst><dgm:cxnLst><dgm:cxn modelId="c1" srcId="0" destId="5" srcOrd="1"/><dgm:cxn modelId="c2" srcId="0" destId="1" srcOrd="0"/><dgm:cxn modelId="c3" type="parOf" srcId="1" destId="2" srcOrd="x"/><dgm:cxn modelId="c4" type="presOf" srcId="1" destId="4"/><dgm:cxn modelId="c5" srcId="2" destId="3" srcOrd="0"/><dgm:cxn modelId="c6" srcId="3" destId="1" srcOrd="0"/><dgm:cxn modelId="c7" srcId="0" destId="6" srcOrd="2"/></dgm:cxnLst></dgm:dataModel>"#;

    fn node(id: &str, level: usize, text: &str) -> DiagramNode {
        DiagramNode {
            id: Some(id.to_owned()),
            level,
            text: text.to_owned(),
        }
    }

    #[test]
    fn a_data_model_reads_as_ooxml_edit_reads_it() {
        let tree = ooxml_xml::parse(MODEL.as_bytes(), &ooxml_xml::Limits::default()).unwrap_or_else(|e| panic!("{e}"));
        let nodes = diagram_nodes(tree.root()).unwrap_or_else(|_| panic!("too deep"));
        assert_eq!(
            nodes,
            [
                node("1", 0, "Goals"),
                node("2", 1, "Faster\u{b}edits\nnow"),
                node("3", 2, "Helper"),
                node("5", 0, ""),
            ]
        );
        assert_eq!(diagram_summary(&nodes), "Goals [Faster edits now [Helper]], ");
    }
}
