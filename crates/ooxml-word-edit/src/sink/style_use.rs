//! Owns: the precise write of [`Key::StyleUse`]: whether a splice or a head change alters,
//! for some paragraph of its part, what a reader of the part's style uses sees -- its
//! paragraph style, its list (`w:numPr`, its `w:numId`) and whether its text is blank --
//! or which such paragraphs there are and in what order. Only the paragraphs the change touches
//! are compared: the paragraph it is made in (the outermost one around it), else what it
//! removes against what it inserts.
//!
//! Must not: read anything but the change and the version it is made on, or decide what an
//! edit means.

use crate::doc::{Change, Key};
use crate::names::W;
use ooxml_xml::{Element, Node, NodePath, Tree};
use std::sync::Arc;

/// What a reader of style uses sees of one paragraph.
#[derive(Debug, PartialEq, Eq)]
struct Use<'a> {
    /// Its `w:pStyle/@w:val`.
    style: Option<&'a str>,
    /// Its `w:numPr` (`None`: none), holding this `w:numId/@w:val`.
    list: Option<Option<&'a str>>,
    /// Whether its `w:t` text has anything but white space.
    text: bool,
}

/// The uses of the paragraphs at or below `nodes` that a reader sees, in document order: a
/// blank paragraph in no list is passed over by both (the body style counts paragraphs with
/// text, a list continued looks for the last numbered one).
fn uses<'a>(nodes: impl IntoIterator<Item = &'a Arc<Element>>) -> Vec<Use<'a>> {
    fn walk<'a>(element: &'a Element, out: &mut Vec<Use<'a>>) {
        if element.name().is(Some(W), "p") {
            let used = use_of(element);
            if used.text || used.list.is_some() {
                out.push(used);
            }
        }
        for child in element.child_elements() {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    for element in nodes {
        walk(element, &mut out);
    }
    out
}

/// What a reader of style uses sees of `paragraph` (its own properties and text).
fn use_of(paragraph: &Element) -> Use<'_> {
    fn val(element: &Arc<Element>) -> Option<&str> {
        element.attribute(Some(W), "val")
    }
    let properties = paragraph.find(Some(W), "pPr");
    let numbering = properties.and_then(|p| p.find(Some(W), "numPr"));
    Use {
        style: properties.and_then(|p| p.find(Some(W), "pStyle")).and_then(val),
        list: numbering.map(|n| n.find(Some(W), "numId").and_then(val)),
        text: has_text(paragraph),
    }
}

/// Whether some `w:t` below `element` holds anything but white space.
fn has_text(element: &Element) -> bool {
    element.child_elements().any(|child| {
        if child.name().is(Some(W), "t") {
            child.text_content().chars().any(|c| !c.is_whitespace())
        } else {
            has_text(child)
        }
    })
}

/// The path of the outermost `w:p` on `path` (the elements from the root down to it), if
/// any.
fn outermost_paragraph(tree: &Tree, path: &NodePath) -> Option<NodePath> {
    let (&first, below) = path.0.split_first()?;
    if first != 0 {
        return None;
    }
    let mut element = tree.root();
    let mut at = vec![0];
    if element.name().is(Some(W), "p") {
        return Some(NodePath(at));
    }
    for &k in below {
        element = element.children().get(k)?.as_element()?;
        at.push(k);
        if element.name().is(Some(W), "p") {
            return Some(NodePath(at));
        }
    }
    None
}

/// The element at `path` of `tree`.
fn element_at<'a>(tree: &'a Tree, path: &NodePath) -> Option<&'a Arc<Element>> {
    let (&first, below) = path.0.split_first()?;
    if first != 0 {
        return None;
    }
    below
        .iter()
        .try_fold(tree.root(), |element, &k| element.children().get(k)?.as_element())
}

/// Whether the paragraph at `paragraph` is seen alike in `before` and `after`.
fn paragraph_alike(before: &Tree, after: &Tree, paragraph: &NodePath) -> bool {
    match (element_at(before, paragraph), element_at(after, paragraph)) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || uses([a]) == uses([b]),
        _ => false,
    }
}

/// The elements of `nodes`.
fn elements(nodes: &[Node]) -> impl Iterator<Item = &Arc<Element>> {
    nodes.iter().filter_map(Node::as_element)
}

/// Whether `change`, made on `tree` (its part's tree in the version it is made on), alters
/// a style use of the part. A change that cannot be applied there alters one (it then
/// fails where it is written anyway).
pub(super) fn alters(tree: &Tree, change: &Change) -> bool {
    match change {
        Change::Splice { patch, .. } => match outermost_paragraph(tree, patch.parent()) {
            Some(paragraph) => !patch
                .apply(tree)
                .is_ok_and(|after| paragraph_alike(tree, &after, &paragraph)),
            None => uses(elements(patch.removed())) != uses(elements(patch.inserted())),
        },
        Change::Head { patch, .. } => match outermost_paragraph(tree, patch.path()) {
            Some(paragraph) => !patch
                .apply(tree)
                .is_ok_and(|after| paragraph_alike(tree, &after, &paragraph)),
            // the head of an element above every paragraph: no paragraph's use changes
            None => false,
        },
        _ => false,
    }
}

/// `change` with [`Key::StyleUse`] of its part added to the keys it writes, where it alters
/// a style use there ([`alters`]); `tree` is the part's tree in the version it is made on.
pub(super) fn keyed(tree: &Tree, change: Change) -> Change {
    let add = |keys: &Arc<[Key]>, part: &Arc<str>| -> Arc<[Key]> {
        let key = Key::StyleUse(Arc::clone(part));
        if keys.contains(&key) {
            return Arc::clone(keys);
        }
        keys.iter().cloned().chain(std::iter::once(key)).collect()
    };
    if !alters(tree, &change) {
        return change;
    }
    match change {
        Change::Splice {
            part,
            patch,
            keys,
            draws,
        } => Change::Splice {
            keys: add(&keys, &part),
            part,
            patch,
            draws,
        },
        Change::Head { part, patch, keys } => Change::Head {
            keys: add(&keys, &part),
            part,
            patch,
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{alters, Change};
    use ooxml_base::Limits;
    use ooxml_xml::{Element, HeadPatch, Node, NodePath, Patch, Tree};
    use std::sync::Arc;

    const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

    fn tree(body: &str) -> Tree {
        let xml = format!("<w:document {NS}><w:body>{body}</w:body></w:document>");
        ooxml_xml::parse(xml.as_bytes(), &Limits::default().xml).unwrap()
    }

    /// The first element of `body`'s content, parsed.
    fn element(body: &str) -> Arc<Element> {
        let parsed = tree(body);
        let body = parsed.root().child_elements().next().unwrap();
        let first = Arc::clone(body.child_elements().next().unwrap());
        first
    }

    fn at<'a>(tree: &'a Tree, path: &[usize]) -> &'a Arc<Element> {
        path.iter()
            .fold(tree.root(), |e, &k| e.children().get(k).unwrap().as_element().unwrap())
    }

    fn splice(tree: &Tree, parent: &[usize], start: usize, new: &str) -> Change {
        let old = at(tree, parent).children().get(start).cloned().unwrap();
        let mut path = vec![0];
        path.extend_from_slice(parent);
        Change::Splice {
            part: Arc::from("word/document.xml"),
            patch: Patch::splice(NodePath(path), start, vec![old], vec![Node::Element(element(new))]),
            keys: Arc::from(Vec::new()),
            draws: Arc::from(Vec::new()),
        }
    }

    const DOC: &str = r#"<w:p><w:r><w:t>Text</w:t></w:r></w:p><w:p/><w:p><w:pPr><w:pStyle w:val="Quote"/></w:pPr><w:r><w:t>q</w:t></w:r></w:p>"#;

    #[test]
    fn a_blank_paragraph_in_no_list_is_seen_by_no_reader() {
        let doc = tree(DOC);
        let styled_blank = r#"<w:p><w:pPr><w:pStyle w:val="Quote"/></w:pPr><w:r><w:t> </w:t></w:r></w:p>"#;
        assert!(!alters(&doc, &splice(&doc, &[0], 1, styled_blank)));
        assert!(alters(
            &doc,
            &splice(&doc, &[0], 1, "<w:p><w:r><w:t>now</w:t></w:r></w:p>")
        ));
        let numbered = r#"<w:p><w:pPr><w:numPr><w:numId w:val="3"/></w:numPr></w:pPr></w:p>"#;
        assert!(alters(&doc, &splice(&doc, &[0], 1, numbered)));
    }

    #[test]
    fn a_change_within_a_paragraph_alters_its_use_only_when_seen() {
        let doc = tree(DOC);
        // the first paragraph's run: other text, then white space only
        assert!(!alters(&doc, &splice(&doc, &[0, 0], 0, "<w:r><w:t>Other</w:t></w:r>")));
        assert!(alters(&doc, &splice(&doc, &[0, 0], 0, "<w:r><w:t>  </w:t></w:r>")));
    }

    #[test]
    fn a_paragraph_style_changed_in_place_alters_a_use() {
        let doc = tree(DOC);
        let style = at(&doc, &[0, 2, 0, 0]);
        let other = element(r#"<w:p><w:pStyle w:val="Normal"/></w:p>"#);
        let new = Arc::clone(other.child_elements().next().unwrap().head());
        let patch = HeadPatch::new(NodePath(vec![0, 0, 2, 0, 0]), Arc::clone(style.head()), new);
        let change = Change::Head {
            part: Arc::from("word/document.xml"),
            patch,
            keys: Arc::from(Vec::new()),
        };
        assert!(alters(&doc, &change));
    }
}
