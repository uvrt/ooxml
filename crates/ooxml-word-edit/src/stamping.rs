//! Owns: giving a document's blocks durable ids before its first edit, as docx-agent's
//! default stamping policy does (`Document._stamp_document`): every paragraph and row, in
//! every story, without a valid and unique `w14:paraId` gets a fresh paraId and textId from
//! its part's sequence, every drawing without an anchorId and editId gets them, and the
//! renames are recorded; a paragraph whose text changed gets a new textId
//! ([`new_text_id`]).
//!
//! Must not: write but through the sink, or stamp a block twice (a stamped block's id is
//! valid, and is left alone).

use crate::doc::Key;
use crate::names::{W14, WP, WP14};
use crate::sink::{ensured_head, EditSink};
use crate::Result;
use ooxml_word_model::docx::{live_paths, Docx, StoryPart};
use ooxml_word_model::ids::{valid_long_hex, IdState, ParagraphId, TableId};
use ooxml_xml::{Attribute, Element, Head, Name, Node, QName};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Attributes to set on an element, in order: each one's name (with the prefix it is
/// written with there) and value.
type Attrs = Vec<(Name, String)>;

/// Stamp every block of every story that needs an id; the renames, in the order made.
///
/// # Errors
/// When a part is not XML.
pub(crate) fn stamp_document(sink: &mut EditSink<'_, '_>, docx: &Docx) -> Result<Vec<(String, String)>> {
    let mut renames: Vec<(String, String)> = Vec::new();
    for story in &docx.parts {
        let made = stamp_part(sink, story)?;
        if made.is_empty() {
            continue;
        }
        let known = sink.doc().ids().aliases().clone();
        let fresh: Vec<(String, String)> = made.iter().filter(|(k, _)| !known.contains_key(k)).cloned().collect();
        sink.rename(fresh)?;
        renames.extend(made);
    }
    Ok(renames)
}

/// The tables of `story` with a row without a durable id, each with the places of those
/// rows.
fn unstamped_rows(story: &StoryPart, prefix: &str) -> Vec<(TableId, Vec<usize>)> {
    let stamped = format!("{prefix}tr:");
    story
        .ids
        .tables()
        .iter()
        .filter_map(|table| {
            let rows: Vec<usize> = table
                .rows
                .iter()
                .enumerate()
                .filter(|(_, (_, id))| !id.starts_with(&stamped))
                .map(|(k, _)| k)
                .collect();
            (!rows.is_empty()).then_some((table, rows))
        })
        .collect()
}

/// Two ids drawn from `seed` for the element at `path`, as the attributes `locals` (in
/// `namespace`, written with the prefix in scope there); the attributes and the first id.
fn draw_pair(
    sink: &mut EditSink<'_, '_>,
    seed: &str,
    root: &Element,
    path: &[u32],
    names: (&str, &str, [&str; 2]),
) -> Result<(Attrs, String)> {
    let (namespace, usual, locals) = names;
    let ids = sink.draw(seed, 2)?;
    let prefix = prefix_at(root, path, namespace, usual);
    let attrs: Attrs = locals
        .iter()
        .zip(&ids)
        .map(|(local, value)| {
            (
                Arc::new(QName::new(Some(&prefix), local, Some(namespace))),
                value.clone(),
            )
        })
        .collect();
    Ok((attrs, ids.into_iter().next().unwrap_or_default()))
}

const BLOCK_IDS: (&str, &str, [&str; 2]) = (W14, "w14", ["paraId", "textId"]);
const DRAWING_IDS: (&str, &str, [&str; 2]) = (WP14, "wp14", ["anchorId", "editId"]);

/// Stamp one story part; its renames.
fn stamp_part(sink: &mut EditSink<'_, '_>, story: &StoryPart) -> Result<Vec<(String, String)>> {
    let part = story.part.as_str();
    let prefix = if story.story == "body" {
        String::new()
    } else {
        format!("{}/", story.story)
    };
    // the index counts what lacks an id: a stamped part is not walked
    let (paragraphs, rows): (Vec<ParagraphId>, _) = if story.ids.fully_stamped() {
        (Vec::new(), Vec::new())
    } else {
        let paragraphs = story
            .ids
            .paragraphs()
            .iter()
            .filter(|p| p.state != IdState::Ok)
            .collect();
        (paragraphs, unstamped_rows(story, &prefix))
    };
    // drawings are looked for only in a part whose memoised facts say it has one to stamp
    let drawings = if crate::validate::unstamped_drawings(sink.doc(), part)? {
        live_paths(story.root(), &[], &|e| {
            (e.name().is(Some(WP), "inline") || e.name().is(Some(WP), "anchor"))
                && !(e.attribute(Some(WP14), "anchorId").is_some_and(valid_long_hex)
                    && e.attribute(Some(WP14), "editId").is_some_and(valid_long_hex))
        })
    } else {
        Vec::new()
    };
    if paragraphs.is_empty() && rows.is_empty() && drawings.is_empty() {
        return Ok(Vec::new());
    }
    sink.read(Key::Part(Arc::from(part)));
    let root = story.root();
    let mut sets: Vec<(Box<[u32]>, Attrs)> = Vec::new();
    let mut renames = Vec::new();
    for paragraph in &paragraphs {
        let (attrs, para) = draw_pair(sink, part, root, &paragraph.path, BLOCK_IDS)?;
        sets.push((paragraph.path.clone(), attrs));
        renames.push((paragraph.id.clone(), format!("{prefix}p:{para}")));
    }
    let mut table_renames = Vec::new();
    for (table, k) in rows
        .iter()
        .flat_map(|(table, rows)| rows.iter().map(move |k| (table, k)))
    {
        let Some((path, row_id)) = table.rows.get(*k) else {
            continue;
        };
        let (attrs, para) = draw_pair(sink, &format!("{part}\0row"), root, path, BLOCK_IDS)?;
        sets.push((path.clone(), attrs));
        renames.push((row_id.clone(), format!("{prefix}tr:{para}")));
        if *k == 0 {
            table_renames.push((table.id.clone(), format!("{prefix}t:{para}")));
        }
    }
    renames.extend(table_renames);
    for drawing in &drawings {
        let (attrs, _) = draw_pair(sink, &format!("{part}\0drawing"), root, &drawing.path, DRAWING_IDS)?;
        sets.push((drawing.path.clone(), attrs));
    }
    let tree = story.tree.clone();
    let mut root = rewrite(
        tree.root(),
        &[],
        &sets.iter().map(|(p, s)| (&**p, s)).collect::<BTreeMap<_, _>>(),
    );
    if !paragraphs.is_empty() {
        root = ensured(&root, false);
    }
    root = ensured(&root, !drawings.is_empty());
    sink.put(part, tree.with_root(root))?;
    Ok(renames)
}

/// `root` with `w14` (and `wp14`) declared as docx-agent's `ensure_w14` leaves it.
fn ensured(root: &Arc<Element>, drawings: bool) -> Arc<Element> {
    match ensured_head(root, drawings, &[]) {
        None => Arc::clone(root),
        Some((head, declared)) => {
            let new = Arc::new(Element::from_head(Arc::new(head), root.children().clone()));
            if declared {
                crate::namespaces::cleaned(&new)
            } else {
                new
            }
        }
    }
}

/// The prefix `namespace`'s attributes are written with at the element at `path` below
/// `root`: the nearest declaration on the way down, else the namespace's usual prefix.
fn prefix_at(root: &Element, path: &[u32], namespace: &str, usual: &str) -> String {
    let declared = |e: &Element| {
        e.namespaces()
            .iter()
            .find(|d| &*d.uri == namespace && d.prefix.is_some())
            .and_then(|d| d.prefix.as_deref().map(str::to_owned))
    };
    let mut found = declared(root);
    let mut at = root;
    for k in path {
        let Some(child) = at.children().get(*k as usize).and_then(Node::as_element) else {
            break;
        };
        at = child;
        found = declared(at).or(found);
    }
    found.unwrap_or_else(|| usual.to_owned())
}

/// `element` (at `place`) with the attributes of `sets` set on the elements they name,
/// every unchanged subtree shared.
fn rewrite(element: &Arc<Element>, place: &[u32], sets: &BTreeMap<&[u32], &Attrs>) -> Arc<Element> {
    let below: Vec<u32> = sets
        .keys()
        .filter(|p| p.len() > place.len() && p.starts_with(place))
        .filter_map(|p| p.get(place.len()).copied())
        .collect();
    let mut children = element.children().clone();
    let mut done = std::collections::BTreeSet::new();
    for k in below {
        if !done.insert(k) {
            continue;
        }
        let Some(Node::Element(child)) = element.children().get(k as usize) else {
            continue;
        };
        let mut path = place.to_vec();
        path.push(k);
        let edited = rewrite(child, &path, sets);
        if let Some(spliced) = children.splice(k as usize..k as usize + 1, vec![Node::Element(edited)]) {
            children = spliced;
        }
    }
    match sets.get(place) {
        Some(attributes) => {
            let mut head: Head = (**element.head()).clone();
            for (name, value) in attributes.iter() {
                head = with_attribute(&head, name, value);
            }
            Arc::new(Element::from_head(Arc::new(head), children))
        }
        None => Arc::new(element.with_children(children)),
    }
}

/// `head` with the attribute `name` set to `value`: in its place when the element has it,
/// else after the others.
pub(crate) fn with_attribute(head: &Head, name: &Name, value: &str) -> Head {
    let mut attributes = head.attributes.to_vec();
    let local = name.local();
    match attributes.iter_mut().find(|a| a.name.is(name.namespace(), local)) {
        Some(a) => a.value = Arc::from(value),
        None => attributes.push(Attribute {
            name: Arc::clone(name),
            value: Arc::from(value),
        }),
    }
    Head {
        name: head.name.clone(),
        namespaces: head.namespaces.clone(),
        attributes: attributes.into(),
    }
}

/// The prefix the paragraph at `path` of `root` writes `w14` attributes with.
pub(crate) fn w14_prefix(root: &Element, path: &[u32]) -> String {
    prefix_at(root, path, W14, "w14")
}

/// The prefix the element at `path` of `root` writes `wp14` attributes with.
pub(crate) fn wp14_prefix(root: &Element, path: &[u32]) -> String {
    prefix_at(root, path, WP14, "wp14")
}
