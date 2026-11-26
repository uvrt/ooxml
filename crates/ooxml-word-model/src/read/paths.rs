//! Owns: finding again the element a layout path names (docx2svg's `paths.resolve_path`):
//! the paths [`super::Steps`] and the run, drawing and text-box readers write for what the
//! layout draws -- a paragraph's `w:body/w:tbl[1]/w:tr[2]/w:tc[1]/w:p[1]`, a text box's
//! `.../w:r[1]/wp:inline[1]/wps:txbx/w:txbxContent/w:p[1]`, a note's
//! `w:footnote[@w:id=2]/w:p[1]` -- with the element's place (child indexes from the root,
//! every node counted), which a part's ids are looked up by.
//!
//! The rules are docx2svg's: `name[k]` is the k-th child with that local name, whatever its
//! namespace; `w:tr[k]`/`w:tc[k]` count through row- and cell-level containers (`w:sdt`'s
//! `w:sdtContent`, `w:customXml`); `wp:anchor[k]`/`wp:inline[k]` after a run are its k-th
//! floating and inline DrawingML object; a group's members count among the group's own
//! children; a step without an index is the first element of that name below (the root
//! itself as the first step); `label` (a list label) is its paragraph; `w:sectPr[k]` as
//! the first step of a main part the k-th section's.
//!
//! A part's ids ([`PartIds`]) find an indexed step among its block container's children
//! in O(log n) ([`resolve_in`]): a body's paragraphs are read one path after another, and
//! counting each one's siblings again would make reading every paragraph of a long body
//! quadratic.
//!
//! Must not: change the tree, or name what a path is drawn for (the part's ids do).

use super::drawing::{drawing_element, object_mark};
use super::runs::RUN_OBJECTS;
use crate::ids::PartIds;
use crate::xmlread::{attr, child, local};
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// The child indexes of `parent`'s children named `name` (by local name), in order.
fn named<'a>(parent: &'a Arc<Element>, name: &'a str) -> impl Iterator<Item = u32> + 'a {
    indexed(parent).filter(move |(_, c)| local(c) == name).map(|(j, _)| j)
}

/// One step of a path: its name (local), its 1-based index, a note's id; `prefixed` false
/// for a pseudo-step (`label`).
struct Step<'a> {
    prefixed: bool,
    name: &'a str,
    index: Option<usize>,
    note: Option<&'a str>,
}

fn step(text: &str) -> Option<Step<'_>> {
    let (prefixed, rest) = match text.split_once(':') {
        Some((prefix, rest)) if word(prefix) => (true, rest),
        Some(_) => return None,
        None => (false, text),
    };
    let (name, tail) = rest.split_once('[').map_or((rest, None), |(n, t)| (n, Some(t)));
    if !word(name) {
        return None;
    }
    let (mut index, mut note) = (None, None);
    if let Some(tail) = tail {
        let inner = tail.strip_suffix(']')?;
        if let Some(id) = inner.strip_prefix("@w:id=") {
            let digits = id.strip_prefix('-').unwrap_or(id);
            (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then_some(())?;
            note = Some(id);
        } else {
            (!inner.is_empty() && inner.bytes().all(|b| b.is_ascii_digit())).then_some(())?;
            index = Some(inner.parse().ok()?);
        }
    }
    Some(Step {
        prefixed,
        name,
        index,
        note,
    })
}

fn word(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// An element's child elements with their node indexes.
fn indexed(element: &Arc<Element>) -> impl Iterator<Item = (u32, &Arc<Element>)> + '_ {
    element
        .children()
        .iter()
        .enumerate()
        .filter_map(|(k, node)| match node {
            Node::Element(e) => Some((u32::try_from(k).ok()?, e)),
            _ => None,
        })
}

/// Where `found` is among `parent`'s descendants: the child indexes down to it.
fn place_below(parent: &Arc<Element>, found: &Element) -> Option<Vec<u32>> {
    for (k, node) in indexed(parent) {
        if std::ptr::eq(&**node, found) {
            return Some(vec![k]);
        }
        if let Some(mut below) = place_below(node, found) {
            below.insert(0, k);
            return Some(below);
        }
    }
    None
}

/// The first descendant of `node` (not itself) named `name`, in document order, with its
/// child indexes below `node`.
fn first_below<'a>(node: &'a Arc<Element>, name: &str) -> Option<(&'a Arc<Element>, Vec<u32>)> {
    for (k, child) in indexed(node) {
        if local(child) == name {
            return Some((child, vec![k]));
        }
        if let Some((found, mut below)) = first_below(child, name) {
            below.insert(0, k);
            return Some((found, below));
        }
    }
    None
}

/// `parent`'s `w:tr` or `w:tc` children, those inside a `w:sdt`'s `w:sdtContent` or a
/// `w:customXml` too, each with its child indexes below `parent` (`row_level`).
fn row_level<'a>(parent: &'a Arc<Element>, name: &str, at: &[u32], out: &mut Vec<(&'a Arc<Element>, Vec<u32>)>) {
    for (k, node) in indexed(parent) {
        let mut here = at.to_vec();
        here.push(k);
        match local(node) {
            n if n == name => out.push((node, here)),
            "sdt" => {
                if let Some((j, content)) = indexed(node).find(|(_, c)| local(c) == "sdtContent") {
                    here.push(j);
                    row_level(content, name, &here, out);
                }
            }
            "customXml" => row_level(node, name, &here, out),
            _ => {}
        }
    }
}

/// A run's floating (`anchor`) or inline (`inline`) DrawingML objects, in order, each the
/// `wp:anchor` or `wp:inline` element (`_anchors`, `_inlines`).
fn run_drawings<'a>(run: &'a Arc<Element>, name: &str) -> Vec<&'a Element> {
    indexed(run)
        .filter(|(_, node)| RUN_OBJECTS.contains(&local(node)))
        .filter_map(|(_, node)| {
            let drawing = drawing_element(node)?;
            match name {
                "anchor" => child(drawing, "anchor"),
                _ if object_mark(node).starts_with("drawing:") => child(drawing, "inline"),
                _ => None,
            }
        })
        .collect()
}

/// Where a drawing's group members are counted: the group an anchor or inline drawing
/// shows, else the node itself (`_members`).
fn members(node: &Arc<Element>) -> Option<(&Arc<Element>, Vec<u32>)> {
    if !matches!(local(node), "anchor" | "inline") {
        return None;
    }
    let (g, graphic) = indexed(node).find(|(_, c)| local(c) == "graphic")?;
    let (d, data) = indexed(graphic).find(|(_, c)| local(c) == "graphicData")?;
    let (c, content) = indexed(data).next()?;
    matches!(local(content), "wgp" | "grpSp").then(|| (content, vec![g, d, c]))
}

/// Every section's `w:sectPr` of a main part's `root`, in document order: each
/// paragraph's (`w:pPr/w:sectPr`), then the body's own (`_sections`).
fn sections(root: &Arc<Element>) -> Vec<(&Arc<Element>, Vec<u32>)> {
    fn walk<'a>(node: &'a Arc<Element>, at: &mut Vec<u32>, out: &mut Vec<(&'a Arc<Element>, Vec<u32>)>) {
        for (k, child) in indexed(node) {
            at.push(k);
            if local(node) == "pPr" && local(child) == "sectPr" {
                out.push((child, at.clone()));
            }
            walk(child, at, out);
            at.pop();
        }
    }
    let mut out = Vec::new();
    walk(root, &mut Vec::new(), &mut out);
    if let Some((b, body)) = indexed(root).find(|(_, c)| local(c) == "body") {
        if let Some((s, last)) = indexed(body).find(|(_, c)| local(c) == "sectPr") {
            out.push((last, vec![b, s]));
        }
    }
    out
}

/// The element `path` names in the part whose root element is `root`, with its place
/// (child indexes from `root`, every node counted); `None` for a path the part does not
/// hold (an edited tree, a synthetic step such as Word's own footnote separator).
#[must_use]
pub fn resolve_path<'a>(root: &'a Arc<Element>, path: &str) -> Option<(&'a Arc<Element>, Vec<u32>)> {
    resolve_within(root, path, None)
}

/// [`resolve_path`] in the story part whose root is `root` and whose ids are `ids`: an
/// indexed step among the part's block container's children found from the ids' counts.
#[must_use]
pub fn resolve_in<'a>(root: &'a Arc<Element>, path: &str, ids: &PartIds) -> Option<(&'a Arc<Element>, Vec<u32>)> {
    resolve_within(root, path, Some(ids))
}

fn resolve_within<'a>(
    root: &'a Arc<Element>,
    path: &str,
    ids: Option<&PartIds>,
) -> Option<(&'a Arc<Element>, Vec<u32>)> {
    if path.is_empty() {
        return None;
    }
    let steps: Vec<Step<'_>> = path.split('/').map(step).collect::<Option<_>>()?;
    let mut node = root;
    let mut place: Vec<u32> = Vec::new();
    for (number, step) in steps.iter().enumerate() {
        if !step.prefixed {
            if step.name == "label" && local(node) == "p" {
                continue;
            }
            return None;
        }
        let (next, below) = if number == 0 && step.name == "sectPr" && local(root) == "document" {
            let k = step.index?;
            sections(root).into_iter().nth(k.checked_sub(1)?)?
        } else if let Some(id) = step.note {
            indexed(node)
                .find(|(_, c)| local(c) == step.name && attr(c, "id") == Some(id))
                .map(|(k, c)| (c, vec![k]))?
        } else if let Some(k) = step.index {
            let k = k.checked_sub(1)?;
            match step.name {
                "tr" | "tc" => {
                    let mut found = Vec::new();
                    row_level(node, step.name, &[], &mut found);
                    found.into_iter().nth(k)?
                }
                "anchor" | "inline" if local(node) == "r" => {
                    let wanted = *run_drawings(node, step.name).get(k)?;
                    let at = place_below(node, wanted)?;
                    (descend(node, &at)?, at)
                }
                _ => {
                    let (base, mut at) = members(node).unwrap_or((node, Vec::new()));
                    let container = ids.filter(|ids| at.is_empty() && place == ids.container());
                    let j = match container {
                        Some(ids) => u32::try_from(ids.container_child(step.name, k + 1)?).ok()?,
                        None => named(base, step.name).nth(k)?,
                    };
                    let found = base.children().get(j as usize)?.as_element()?;
                    at.push(j);
                    (found, at)
                }
            }
        } else {
            if number == 0 && local(node) == step.name {
                continue;
            }
            first_below(node, step.name)?
        };
        node = next;
        place.extend(below);
    }
    Some((node, place))
}

/// The element at child indexes `at` below `node`.
fn descend<'a>(mut node: &'a Arc<Element>, at: &[u32]) -> Option<&'a Arc<Element>> {
    for k in at {
        node = match node.children().get(*k as usize)? {
            Node::Element(e) => e,
            _ => return None,
        };
    }
    Some(node)
}

#[cfg(test)]
mod tests {
    use super::resolve_path;
    use ooxml_base::XmlLimits;
    use ooxml_xml::Element;
    use std::sync::Arc;

    const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006""#;

    fn parse(xml: &str) -> Arc<Element> {
        let tree = ooxml_xml::parse(xml.as_bytes(), &XmlLimits::default()).unwrap_or_else(|e| panic!("{e}"));
        Arc::clone(tree.root())
    }

    fn root(body: &str) -> Arc<Element> {
        let xml = format!("<w:document {NS}><w:body>{body}</w:body></w:document>");
        parse(&xml)
    }

    /// Every text node below `found`, in order.
    fn text_of(found: &Element) -> String {
        found
            .children()
            .iter()
            .map(|node| match node {
                ooxml_xml::Node::Text(t) => t.to_string(),
                ooxml_xml::Node::Element(e) => text_of(e),
                _ => String::new(),
            })
            .collect()
    }

    const BOX: &str =
        "<a:graphic><a:graphicData><wps:wsp><wps:txbx><w:txbxContent><w:p><w:r><w:t>one</w:t></w:r></w:p>\
        <w:p><w:r><w:t>two</w:t></w:r></w:p></w:txbxContent></wps:txbx></wps:wsp></a:graphicData></a:graphic>";

    #[test]
    fn a_text_box_paragraph_in_a_cell() {
        let doc = root(&format!(
            "<w:p/><w:tbl><w:sdt><w:sdtContent><w:tr><w:tc><w:p><w:r><w:drawing><wp:anchor>{BOX}</wp:anchor></w:drawing>\
             <mc:AlternateContent><mc:Choice><w:drawing><wp:inline><wp:extent cx=\"1\" cy=\"1\"/>{BOX}</wp:inline>\
             </w:drawing></mc:Choice></mc:AlternateContent></w:r></w:p></w:tc></w:tr></w:sdtContent></w:sdt></w:tbl>"
        ));
        let cell = "w:body/w:tbl[1]/w:tr[1]/w:tc[1]/w:p[1]/w:r[1]";
        let (found, place) =
            resolve_path(&doc, &format!("{cell}/wp:anchor[1]/wps:txbx/w:txbxContent/w:p[2]")).expect("found");
        assert_eq!(text_of(found), "two");
        assert_eq!(place.len(), 16, "{place:?}");
        let (found, _) =
            resolve_path(&doc, &format!("{cell}/wp:inline[1]/wps:txbx/w:txbxContent/w:p[1]")).expect("found");
        assert_eq!(text_of(found), "one");
        assert!(resolve_path(&doc, &format!("{cell}/wp:inline[2]/wps:txbx/w:txbxContent/w:p[1]")).is_none());
        let (found, place) = resolve_path(&doc, "w:body/w:p[1]/label").expect("found");
        assert_eq!((found.name().local(), place), ("p", vec![0, 0]));
    }

    #[test]
    fn a_note_by_its_id() {
        let xml = format!(
            "<w:footnotes {NS}><w:footnote w:id=\"-1\"/><w:footnote w:id=\"2\"><w:p><w:r><w:t>n</w:t></w:r></w:p>\
             </w:footnote></w:footnotes>"
        );
        let notes = parse(&xml);
        let (found, place) = resolve_path(&notes, "w:footnote[@w:id=2]/w:p[1]").expect("found");
        assert_eq!((text_of(found).as_str(), place), ("n", vec![1, 0]));
        assert!(resolve_path(&notes, "w:footnotes/separator").is_none());
    }

    #[test]
    fn a_long_body_s_paths_are_found_by_its_ids_as_by_counting() {
        let paragraphs: String = (0..150)
            .map(|k| {
                if k % 7 == 0 {
                    "<w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>".to_owned()
                } else {
                    format!("<w:p><w:r><w:t>{k}</w:t></w:r></w:p>")
                }
            })
            .collect();
        let doc = parse(&format!(
            "<w:document {NS}><w:body>{paragraphs}<w:sectPr/></w:body></w:document>"
        ));
        let ids = crate::ids::PartIds::new("body", &doc);
        for k in 1..=140 {
            for path in [
                format!("w:body/w:p[{k}]"),
                format!("w:body/w:p[{k}]/w:r[1]"),
                format!("w:body/w:tbl[{}]/w:tr[1]/w:tc[1]/w:p[1]", 1 + k % 22),
            ] {
                let indexed = super::resolve_in(&doc, &path, &ids).map(|(e, at)| (Arc::as_ptr(e), at));
                let counted = resolve_path(&doc, &path).map(|(e, at)| (Arc::as_ptr(e), at));
                assert_eq!(indexed, counted, "{path}");
            }
        }
        assert!(super::resolve_in(&doc, "w:body/w:p[200]", &ids).is_none());
    }
}
