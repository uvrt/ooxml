//! Owns: a DrawingML text body given new text with its formatting kept character by
//! character, as ooxml-edit's `charts/dmltext.py` gives it (`replace_body_text`): the
//! paragraphs diffed by their text (`difflib`'s opcodes), each paired paragraph rebuilt from
//! its old runs (`rebuild_paragraph`: characters kept, replaced or put in take the
//! properties of what they keep, replace or follow; a field left whole stays a field; a line
//! break is `\v`), new paragraphs made like their neighbour (`paragraph_like`) and the rest
//! taken out.
//!
//! Must not: know what the body belongs to (a chart title, an axis title).

use ooxml_base::difflib::{coarse, opcodes, Tag};
use ooxml_xml::view::{local, text};
use ooxml_xml::{Atom, Element, Node, QName};
use std::sync::Arc;

/// A line break in a paragraph's text (`LINE_BREAK`).
const LINE_BREAK: char = '\u{b}';
/// DrawingML's namespace.
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

fn is_a(element: &Element, name: &str) -> bool {
    element.name().namespace() == Some(A) && local(element) == name
}

/// A run or a field (`RUN_TAGS`).
fn is_run(element: &Element) -> bool {
    is_a(element, "r") || is_a(element, "fld")
}

fn a_child<'e>(element: &'e Element, name: &str) -> Option<&'e Arc<Element>> {
    element.child_elements().find(|e| is_a(e, name))
}

/// A run's (or field's) text: its `a:t`'s.
fn run_text(run: &Element) -> String {
    a_child(run, "t").map(|t| text(t)).unwrap_or_default()
}

/// `paragraph_text`: a paragraph's text, a line break as `\v`.
#[must_use]
pub fn paragraph_text(paragraph: &Element) -> String {
    let mut out = String::new();
    for child in paragraph.child_elements() {
        if is_run(child) {
            out.push_str(&run_text(child));
        } else if is_a(child, "br") {
            out.push(LINE_BREAK);
        }
    }
    out
}

/// A new element `local_name` named as `like` is (its prefix and namespace).
fn like(like: &Element, local_name: &str, attributes: Vec<ooxml_xml::Attribute>, children: Vec<Node>) -> Element {
    let name = Arc::new(QName::new(like.name().prefix(), local_name, like.name().namespace()));
    Element::new(name, Vec::new(), attributes, children)
}

/// `properties` copied as an `a:rPr` (an `a:endParaRPr` is the same type).
fn as_run_properties(properties: &Element) -> Element {
    let name = Arc::new(QName::new(
        properties.name().prefix(),
        "rPr",
        properties.name().namespace(),
    ));
    Element::new(
        name,
        properties.namespaces().to_vec(),
        properties.attributes().to_vec(),
        properties.children().to_vec(),
    )
}

/// `make_run`.
fn make_run(paragraph: &Element, piece: &str, properties: Option<&Element>) -> Element {
    let mut children = Vec::new();
    if let Some(properties) = properties {
        children.push(Node::Element(Arc::new(as_run_properties(properties))));
    }
    let t = like(paragraph, "t", Vec::new(), vec![Node::Text(Atom::from(piece))]);
    children.push(Node::Element(Arc::new(t)));
    like(paragraph, "r", Vec::new(), children)
}

/// `make_break`.
fn make_break(paragraph: &Element, properties: Option<&Element>) -> Element {
    let children = properties
        .map(|p| vec![Node::Element(Arc::new(as_run_properties(p)))])
        .unwrap_or_default();
    like(paragraph, "br", Vec::new(), children)
}

/// `paragraph_like`: a new paragraph formatted like `template` (its `a:pPr`, its last run's
/// properties, its end's).
fn paragraph_like(template: &Element, value: &str) -> Element {
    let mut children = Vec::new();
    if let Some(properties) = a_child(template, "pPr") {
        children.push(Node::Element(Arc::clone(properties)));
    }
    let last_run = template.child_elements().filter(|e| is_run(e)).last();
    let end = a_child(template, "endParaRPr");
    let properties = last_run.and_then(|r| a_child(r, "rPr")).or(end).map(|p| &**p);
    for (index, piece) in value.split(LINE_BREAK).enumerate() {
        if index > 0 {
            children.push(Node::Element(Arc::new(make_break(template, properties))));
        }
        if !piece.is_empty() {
            children.push(Node::Element(Arc::new(make_run(template, piece, properties))));
        }
    }
    if let Some(end) = end {
        children.push(Node::Element(Arc::clone(end)));
    }
    like(template, "p", Vec::new(), children)
}

/// A run, field or break of a paragraph (`_Item`).
struct Item<'p> {
    element: &'p Arc<Element>,
    is_break: bool,
    is_field: bool,
    start: usize,
    text: Vec<char>,
}

/// `inline_items`: the runs, fields and breaks, and the other inline elements by offset.
fn inline_items(paragraph: &Element) -> (Vec<Item<'_>>, Vec<(usize, Arc<Element>)>) {
    let (mut items, mut others) = (Vec::new(), Vec::new());
    let mut offset = 0;
    for child in paragraph.child_elements() {
        if is_run(child) {
            let text: Vec<char> = run_text(child).chars().collect();
            let length = text.len();
            items.push(Item {
                element: child,
                is_break: false,
                is_field: is_a(child, "fld"),
                start: offset,
                text,
            });
            offset += length;
        } else if is_a(child, "br") {
            items.push(Item {
                element: child,
                is_break: true,
                is_field: false,
                start: offset,
                text: vec![LINE_BREAK],
            });
            offset += 1;
        } else if !is_a(child, "pPr") && !is_a(child, "endParaRPr") {
            others.push((offset, Arc::clone(child)));
        }
    }
    (items, others)
}

/// `_text_properties`: the run properties for text from item `index` (none: the end's).
fn text_properties<'p>(paragraph: &'p Element, items: &[Item<'p>], index: Option<usize>) -> Option<&'p Element> {
    let Some(index) = index else {
        return a_child(paragraph, "endParaRPr").map(|e| &**e);
    };
    let properties = |i: usize| items.get(i).and_then(|item| a_child(item.element, "rPr")).map(|e| &**e);
    let item = items.get(index)?;
    if !item.is_break {
        return properties(index);
    }
    let nearest = (0..index).rev().chain(index + 1..items.len());
    for step in nearest {
        if items.get(step).is_some_and(|i| !i.is_break) {
            return properties(step);
        }
    }
    properties(index)
}

/// `_replace_inline`: the paragraph's inline content swapped for `built`, before its
/// `a:endParaRPr` (the paragraph's leading text, its `a:pPr` and its end kept, with the text
/// after them).
fn replace_inline(paragraph: &Element, built: Vec<(usize, Element)>) -> Element {
    let mut kept: Vec<Node> = Vec::new();
    let mut keeping = true; // the text before the first child is the paragraph's own
    let mut end_at = None;
    for node in paragraph.children().iter() {
        match node {
            Node::Element(e) => {
                keeping = is_a(e, "pPr") || is_a(e, "endParaRPr");
                if keeping {
                    if is_a(e, "endParaRPr") && end_at.is_none() {
                        end_at = Some(kept.len());
                    }
                    kept.push(node.clone());
                }
            }
            Node::Text(_) if keeping => kept.push(node.clone()),
            _ => keeping = false,
        }
    }
    let new: Vec<Node> = built.into_iter().map(|(_, e)| Node::Element(Arc::new(e))).collect();
    let at = end_at.unwrap_or(kept.len());
    kept.splice(at..at, new);
    paragraph.with_children(ooxml_xml::Children::from_vec(kept))
}

/// For each new character its source item and the old character it keeps, and where each
/// old character went (`rebuild_paragraph`'s first pass over the coarse opcodes).
#[allow(clippy::type_complexity)]
fn sources_of(
    old: &[char],
    new: &[char],
    owner: &[usize],
) -> (
    Vec<(Option<usize>, Option<usize>)>,
    std::collections::HashMap<usize, usize>,
) {
    // for each new character: its source item, and the old character it keeps
    let mut sources: Vec<(Option<usize>, Option<usize>)> = Vec::new();
    let mut new_offset_of_old: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for op in coarse(old, new) {
        if op.tag == Tag::Equal {
            for k in 0..op.i2 - op.i1 {
                sources.push((owner.get(op.i1 + k).copied(), Some(op.i1 + k)));
                new_offset_of_old.insert(op.i1 + k, op.j1 + k);
            }
            continue;
        }
        let source = if op.i1 < op.i2 {
            owner.get(op.i1).copied()
        } else if op.i1 > 0 {
            owner.get(op.i1 - 1).copied()
        } else {
            owner.first().copied()
        };
        for _ in op.j1..op.j2 {
            sources.push((source, None));
        }
        for k in op.i1..op.i2 {
            new_offset_of_old.insert(k, op.j1);
        }
    }
    (sources, new_offset_of_old)
}

/// `rebuild_paragraph`: `paragraph` with the text `value`, its formatting kept character by
/// character.
#[must_use]
pub fn rebuild_paragraph(paragraph: &Element, value: &str) -> Element {
    let (items, others) = inline_items(paragraph);
    let old: Vec<char> = items.iter().flat_map(|i| i.text.iter().copied()).collect();
    let new: Vec<char> = value.chars().collect();
    if old == new {
        return paragraph.clone();
    }
    let owner: Vec<usize> = items
        .iter()
        .enumerate()
        .flat_map(|(index, item)| std::iter::repeat_n(index, item.text.len()))
        .collect();
    let (sources, new_offset_of_old) = sources_of(&old, &new, &owner);
    let mut built: Vec<(usize, Element)> = Vec::new();
    let mut position = 0;
    while position < new.len() {
        let (source, kept) = sources.get(position).copied().unwrap_or((None, None));
        let item = source.and_then(|s| items.get(s));
        if new.get(position) == Some(&LINE_BREAK) {
            let element = match item {
                Some(item) if item.is_break && kept.is_some() => (**item.element).clone(),
                _ => make_break(paragraph, text_properties(paragraph, &items, source)),
            };
            built.push((position, element));
            position += 1;
            continue;
        }
        let mut end = position;
        while end < new.len() && new.get(end) != Some(&LINE_BREAK) && sources.get(end).map(|s| s.0) == Some(source) {
            end += 1;
        }
        let piece: String = new.get(position..end).unwrap_or_default().iter().collect();
        let kept_span: Vec<Option<usize>> = (position..end).map(|k| sources.get(k).and_then(|s| s.1)).collect();
        let whole_field = item.is_some_and(|field| {
            field.is_field
                && kept_span
                    .iter()
                    .copied()
                    .eq((field.start..field.start + field.text.len()).map(Some))
        });
        let element = match item {
            Some(field) if whole_field => (**field.element).clone(),
            _ => make_run(paragraph, &piece, text_properties(paragraph, &items, source)),
        };
        built.push((position, element));
        position = end;
    }
    for (old_offset, element) in others {
        let mapped = new_offset_of_old
            .get(&old_offset)
            .copied()
            .unwrap_or(if old_offset >= old.len() { new.len() } else { 0 });
        built.push((mapped, (*element).clone()));
    }
    built.sort_by_key(|(offset, _)| *offset); // stable: objects before text at one offset
    replace_inline(paragraph, built)
}

/// An empty body (`c:rich`) given an `a:bodyPr`, an `a:lstStyle` (in its sequence) and a
/// paragraph, named with `prefix`.
fn give_paragraph(nodes: &mut Vec<Node>, prefix: Option<&str>) {
    let a_like = |name: &str| {
        Element::new(
            Arc::new(QName::new(prefix, name, Some(A))),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    };
    for (name, rank) in [("bodyPr", 0), ("lstStyle", 1)] {
        let has = nodes.iter().any(|n| n.as_element().is_some_and(|e| is_a(e, name)));
        if !has {
            let made = Node::Element(Arc::new(a_like(name)));
            // `c:rich`'s sequence: bodyPr, lstStyle, p
            let at = nodes
                .iter()
                .position(|n| {
                    n.as_element().is_some_and(|e| match local(e) {
                        "bodyPr" => false,
                        "lstStyle" => rank < 1,
                        _ => true,
                    })
                })
                .unwrap_or(nodes.len());
            nodes.insert(at, made);
        }
    }
    nodes.push(Node::Element(Arc::new(a_like("p"))));
}

/// `replace_body_text`: the body `body` (`c:rich`, `a:txBody`) with the text `value` (`\n`
/// between paragraphs), its formatting kept; an empty body gets an `a:bodyPr`, an
/// `a:lstStyle` and a paragraph.
#[must_use]
pub fn replace_body_text(body: &Element, value: &str) -> Element {
    let mut nodes: Vec<Node> = body.children().to_vec();
    let prefix = nodes
        .iter()
        .find_map(|n| n.as_element().filter(|e| e.name().namespace() == Some(A)))
        .map_or(Some("a".to_owned()), |e| e.name().prefix().map(str::to_owned));
    let places = |nodes: &[Node]| -> Vec<usize> {
        nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.as_element().is_some_and(|e| is_a(e, "p")))
            .map(|(i, _)| i)
            .collect()
    };
    if places(&nodes).is_empty() {
        give_paragraph(&mut nodes, prefix.as_deref());
    }
    let paragraphs = places(&nodes);
    let old: Vec<String> = paragraphs
        .iter()
        .filter_map(|&at| nodes.get(at).and_then(Node::as_element))
        .map(|p| paragraph_text(p))
        .collect();
    let new: Vec<String> = value.split('\n').map(str::to_owned).collect();
    for op in opcodes(&old, &new).into_iter().rev() {
        if op.tag == Tag::Equal {
            continue;
        }
        let old_block: Vec<usize> = paragraphs.get(op.i1..op.i2).unwrap_or_default().to_vec();
        let new_block = new.get(op.j1..op.j2).unwrap_or_default();
        let paired = old_block.len().min(new_block.len());
        for (at, value) in old_block.iter().zip(new_block) {
            if let Some(p) = nodes.get(*at).and_then(Node::as_element).map(Arc::clone) {
                if let Some(slot) = nodes.get_mut(*at) {
                    *slot = Node::Element(Arc::new(rebuild_paragraph(&p, value)));
                }
            }
        }
        if new_block.len() > paired {
            let (template, anchor, after) = match (old_block.last(), op.i1.checked_sub(1)) {
                (Some(&last), _) => (last, last, true),
                (None, Some(before)) => {
                    let at = paragraphs.get(before).copied().unwrap_or(0);
                    (at, at, true)
                }
                (None, None) => {
                    let at = paragraphs.first().copied().unwrap_or(0);
                    (at, at, false)
                }
            };
            let Some(template) = nodes.get(template).and_then(Node::as_element).map(Arc::clone) else {
                continue;
            };
            let extras: Vec<Node> = new_block
                .get(paired..)
                .unwrap_or_default()
                .iter()
                .map(|v| Node::Element(Arc::new(paragraph_like(&template, v))))
                .collect();
            let at = if after {
                // after the anchor and its tail (lxml's `addnext`)
                (anchor + 1..nodes.len())
                    .find(|&i| !matches!(nodes.get(i), Some(Node::Text(_))))
                    .unwrap_or(nodes.len())
            } else {
                anchor
            };
            nodes.splice(at..at, extras);
        }
        // the paragraphs left over go, last first so the places stay true (their text stays)
        for &at in old_block.get(paired..).unwrap_or_default().iter().rev() {
            nodes.remove(at);
        }
    }
    body.with_children(ooxml_xml::Children::from_vec(nodes))
}

#[cfg(test)]
mod tests {
    //! Against ooxml-edit's `replace_body_text` on the same body (worked out with it,
    //! 2026-11-21): unchanged, a word replaced in a bold run, a word in an italic run, a
    //! paragraph added like the last, a paragraph taken out, all text replaced, emptied.
    use super::replace_body_text;
    use ooxml_xml::Limits;

    const HEAD: &str = r#"<c:rich xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:bodyPr/><a:lstStyle/>"#;
    const FIRST: &str = r#"<a:p><a:pPr/><a:r><a:rPr b="1"/><a:t>Sales </a:t></a:r><a:r><a:rPr i="1"/><a:t>by region</a:t></a:r><a:br><a:rPr/></a:br><a:fld id="{1}" type="x"><a:rPr/><a:t>2024</a:t></a:fld><a:endParaRPr lang="en-GB"/></a:p>"#;

    fn rewritten(value: &str) -> String {
        let body = format!("{HEAD}{FIRST}<a:p><a:r><a:t>Second</a:t></a:r></a:p></c:rich>");
        let tree = ooxml_xml::parse(body.as_bytes(), &Limits::default()).unwrap_or_else(|e| panic!("{e}"));
        let written = replace_body_text(tree.root(), value);
        let out = ooxml_xml::Tree::from_root(std::sync::Arc::new(written)).write();
        let out = String::from_utf8(out).unwrap_or_default();
        out.split_once("?>\n").map_or(out.clone(), |(_, rest)| rest.to_owned())
    }

    #[test]
    fn text_is_written_with_its_formatting_kept() {
        let second = "<a:p><a:r><a:t>Second</a:t></a:r></a:p>";
        assert_eq!(
            rewritten("Sales by region\u{b}2024\nSecond"),
            format!("{HEAD}{FIRST}{second}</c:rich>")
        );
        let revenue = FIRST.replace("Sales ", "Revenue ");
        assert_eq!(
            rewritten("Revenue by region\u{b}2024"),
            format!("{HEAD}{revenue}</c:rich>")
        );
        let country = FIRST.replace("by region", "by country");
        assert_eq!(
            rewritten("Sales by country\u{b}2024\nSecond\nThird"),
            format!("{HEAD}{country}{second}<a:p><a:r><a:t>Third</a:t></a:r></a:p></c:rich>")
        );
        assert_eq!(
            rewritten("New"),
            format!(
                r#"{HEAD}<a:p><a:pPr/><a:r><a:rPr b="1"/><a:t>New</a:t></a:r><a:endParaRPr lang="en-GB"/></a:p></c:rich>"#
            )
        );
        assert_eq!(
            rewritten(""),
            format!(r#"{HEAD}<a:p><a:pPr/><a:endParaRPr lang="en-GB"/></a:p></c:rich>"#)
        );
    }
}
