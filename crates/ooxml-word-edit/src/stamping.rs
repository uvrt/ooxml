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
use ooxml_word_model::ids::{valid_long_hex, IdState};
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

/// Stamp one story part; its renames.
fn stamp_part(sink: &mut EditSink<'_, '_>, story: &StoryPart) -> Result<Vec<(String, String)>> {
    let part = story.part.as_str();
    let prefix = if story.story == "body" {
        String::new()
    } else {
        format!("{}/", story.story)
    };
    let paragraphs: Vec<_> = story.ids.paragraphs.iter().filter(|p| p.state != IdState::Ok).collect();
    let rows: Vec<(usize, usize)> = story
        .ids
        .tables
        .iter()
        .enumerate()
        .flat_map(|(t, table)| table.rows.iter().enumerate().map(move |(k, _)| (t, k)))
        .filter(|(t, k)| {
            let id = story
                .ids
                .tables
                .get(*t)
                .and_then(|table| table.rows.get(*k))
                .map(|r| r.1.as_str());
            !id.is_some_and(|id| id.starts_with(&format!("{prefix}tr:")))
        })
        .collect();
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
    let named = |path: &[u32], namespace: &str, usual: &str, local: &str| -> Name {
        Arc::new(QName::new(
            Some(&prefix_at(root, path, namespace, usual)),
            local,
            Some(namespace),
        ))
    };
    let mut sets: Vec<(Box<[u32]>, Attrs)> = Vec::new();
    let mut renames = Vec::new();
    for paragraph in &paragraphs {
        let ids = sink.draw(part, 2)?;
        let (para, text) = (
            ids.first().cloned().unwrap_or_default(),
            ids.get(1).cloned().unwrap_or_default(),
        );
        let p = &paragraph.path;
        sets.push((
            p.clone(),
            vec![
                (named(p, W14, "w14", "paraId"), para.clone()),
                (named(p, W14, "w14", "textId"), text),
            ],
        ));
        renames.push((paragraph.id.clone(), format!("{prefix}p:{para}")));
    }
    let mut table_renames = Vec::new();
    for (t, k) in &rows {
        let Some(table) = story.ids.tables.get(*t) else {
            continue;
        };
        let Some((path, row_id)) = table.rows.get(*k) else {
            continue;
        };
        let ids = sink.draw(&format!("{part}\0row"), 2)?;
        let (para, text) = (
            ids.first().cloned().unwrap_or_default(),
            ids.get(1).cloned().unwrap_or_default(),
        );
        sets.push((
            path.clone(),
            vec![
                (named(path, W14, "w14", "paraId"), para.clone()),
                (named(path, W14, "w14", "textId"), text),
            ],
        ));
        renames.push((row_id.clone(), format!("{prefix}tr:{para}")));
        if *k == 0 {
            table_renames.push((table.id.clone(), format!("{prefix}t:{para}")));
        }
    }
    renames.extend(table_renames);
    for drawing in &drawings {
        let ids = sink.draw(&format!("{part}\0drawing"), 2)?;
        let (anchor, edit) = (
            ids.first().cloned().unwrap_or_default(),
            ids.get(1).cloned().unwrap_or_default(),
        );
        let p = &drawing.path;
        sets.push((
            p.clone(),
            vec![
                (named(p, WP14, "wp14", "anchorId"), anchor),
                (named(p, WP14, "wp14", "editId"), edit),
            ],
        ));
    }
    let tree = story.tree.clone();
    let mut root = rewrite(
        tree.root(),
        &[],
        &sets.iter().map(|(p, s)| (&**p, s)).collect::<BTreeMap<_, _>>(),
    );
    if !paragraphs.is_empty() {
        if let Some(head) = ensured_head(&root, false, &[]) {
            root = Arc::new(Element::from_head(Arc::new(head), root.children().clone()));
        }
    }
    if let Some(head) = ensured_head(&root, !drawings.is_empty(), &[]) {
        root = Arc::new(Element::from_head(Arc::new(head), root.children().clone()));
    }
    sink.put(part, tree.with_root(root))?;
    Ok(renames)
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

/// Give the paragraph (whose text changed) a new `w14:textId`, as Word does: the textId
/// versions the text.
///
/// # Errors
/// When the ids cannot be drawn.
pub(crate) fn new_text_id(sink: &mut EditSink<'_, '_>, part: &str) -> Result<String> {
    let ids = sink.draw(&format!("{part}\0text"), 1)?;
    Ok(ids.into_iter().next().unwrap_or_default())
}

/// The prefix the paragraph at `path` of `root` writes `w14` attributes with.
pub(crate) fn w14_prefix(root: &Element, path: &[u32]) -> String {
    prefix_at(root, path, W14, "w14")
}
