//! Owns: the numbering part's writes a list edit makes, as docx-agent's `edit/numbering.py`
//! makes them -- Word's own Bullets and Numbering definitions ([`new_list`]), an instance
//! over a definition ([`add_num`]) and a restart's ([`add_restart`]), a definition copied
//! for copy-on-write ([`copy_abstract`]), an instance pointed at another definition, a
//! level's element changed in place ([`edit_level`]), and the instances and definitions
//! an edit left unused removed ([`reap`]).
//!
//! Must not: write but through the sink, or touch a paragraph (the list commands do).

use crate::doc::Key;
use crate::list_style::parsed;
use crate::names::W;
use crate::sink::EditSink;
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::stamp::namespaces::strip_redundant;
use ooxml_word_model::xmlread::py_int;
use ooxml_xml::{Element, Node};
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

/// The numbering part's content type.
const NUMBERING_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
/// The relationship type of the numbering part.
pub(crate) const REL_NUMBERING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
/// The levels a list has.
pub(crate) const LEVELS: i64 = 9;
/// Word's own bullet list: per level, (character, font, template code).
const BULLETS: [(&str, &str, &str); 3] = [
    ("\u{f0b7}", "Symbol", "04090001"),
    ("o", "Courier New", "04090003"),
    ("\u{f0a7}", "Wingdings", "04090005"),
];
/// Word's own numbered list: per level, (format, alignment, template code).
const NUMBERS: [(&str, &str, &str); 3] = [
    ("decimal", "left", "0409000F"),
    ("lowerLetter", "left", "04090019"),
    ("lowerRoman", "right", "0409001B"),
];
const W_DECL: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

/// The numbering part of `main`, if it has one.
///
/// # Errors
/// When the relationships are not XML.
pub(crate) fn part_of(sink: &EditSink<'_, '_>, main: &str) -> Result<Option<String>> {
    Ok(ooxml_word_model::parts::related(
        sink.doc().package(),
        main,
        REL_NUMBERING,
    )?)
}

/// The numbering part's root (docx-agent's `Numbering.root`).
fn root(sink: &EditSink<'_, '_>, part: &str) -> Result<Arc<Element>> {
    let tree = sink
        .doc()
        .tree(part)?
        .ok_or_else(|| EditError::no_part(part.to_owned()))?;
    Ok(Arc::clone(tree.root()))
}

/// The children `w:{local}` of `root` by their number in `w:{attribute}`, with their
/// places: a later one of a number replaces an earlier one (docx-agent's `_listed`).
pub(crate) fn listed(root: &Element, local: &str, attribute: &str) -> Vec<(i64, usize)> {
    let mut out: Vec<(i64, usize)> = Vec::new();
    for (k, node) in root.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        if !child.name().is(Some(W), local) {
            continue;
        }
        if let Some(n) = child.attribute(Some(W), attribute).and_then(py_int) {
            match out.iter_mut().find(|(m, _)| *m == n) {
                Some(slot) => slot.1 = k,
                None => out.push((n, k)),
            }
        }
    }
    out
}

/// The child element of `root` at `k`.
fn child(root: &Element, k: usize) -> Option<&Arc<Element>> {
    match root.children().get(k) {
        Some(Node::Element(e)) => Some(e),
        _ => None,
    }
}

/// The place of the last child `w:{local}`, and of the first.
fn last_of(root: &Element, local: &str) -> Option<usize> {
    let children = root.children().to_vec();
    children
        .iter()
        .rposition(|n| matches!(n, Node::Element(e) if e.name().is(Some(W), local)))
}

fn first_of(root: &Element, local: &str) -> Option<usize> {
    root.children()
        .iter()
        .position(|n| matches!(n, Node::Element(e) if e.name().is(Some(W), local)))
}

/// The numbering part, made as Word declares it when the document has none
/// (docx-agent's `_ensure_part`).
fn ensure_part(sink: &mut EditSink<'_, '_>, main: &str) -> Result<String> {
    let part = match part_of(sink, main)? {
        Some(part) => part,
        None => sink.new_part(
            main,
            ("word/numbering.xml", "word/numbering{n}.xml"),
            crate::new_part::word_root("numbering"),
            NUMBERING_TYPE,
            REL_NUMBERING,
        )?,
    };
    sink.read(part_key(&part));
    Ok(part)
}

/// The key a numbering write reads and writes: the whole part.
pub(crate) fn part_key(part: &str) -> Key {
    Key::part(part)
}

/// docx-agent's `_nsid`: SHA-256(`seed\0counter`)'s eight leading hex digits, in upper
/// case, the first not in `used` and not `00000000`.
pub(crate) fn nsid(seed: &str, used: &HashSet<String>) -> String {
    (0u64..)
        .map(|counter| {
            let digest = ooxml_word_model::ids::sha256(format!("{seed}\0{counter}").as_bytes());
            digest.iter().take(4).map(|b| format!("{b:02X}")).collect::<String>()
        })
        .find(|v| !used.contains(v) && v != "00000000")
        .unwrap_or_default()
}

/// Every `w:nsid` value below `element`, in upper case.
fn nsids(element: &Element, out: &mut HashSet<String>) {
    for child in element.child_elements() {
        if child.name().is(Some(W), "nsid") {
            out.insert(child.attribute(Some(W), "val").unwrap_or("").to_uppercase());
        }
        nsids(child, out);
    }
}

/// The `w:nsid` values in use in the numbering part (none without one).
pub(crate) fn used_nsids(sink: &EditSink<'_, '_>, part: Option<&str>) -> Result<HashSet<String>> {
    let mut used = HashSet::new();
    if let Some(part) = part {
        nsids(root(sink, part)?.as_ref(), &mut used);
    }
    Ok(used)
}

/// Word's own Bullets or Numbering definition (docx-agent's `builtin_abstract`).
pub(crate) fn builtin_abstract(kind: &str, abstract_id: i64, nsid: &str, template: &str) -> String {
    let mut levels = String::new();
    for level in 0..LEVELS {
        let indent = 720 * (level + 1);
        let pick = usize::try_from(level % 3).unwrap_or(0);
        if kind == "bullet" {
            let (char, font, code) = BULLETS.get(pick).copied().unwrap_or(BULLETS[0]);
            let cs = if font == "Courier New" {
                r#" w:cs="Courier New""#
            } else {
                ""
            };
            levels.push_str(&format!(
                r#"<w:lvl w:ilvl="{level}" w:tplc="{code}"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="{char}"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="{indent}" w:hanging="360"/></w:pPr><w:rPr><w:rFonts w:ascii="{font}" w:hAnsi="{font}"{cs} w:hint="default"/></w:rPr></w:lvl>"#
            ));
        } else {
            let (format, jc, code) = NUMBERS.get(pick).copied().unwrap_or(NUMBERS[0]);
            let hanging = if jc == "right" { 180 } else { 360 };
            let number = level + 1;
            levels.push_str(&format!(
                r#"<w:lvl w:ilvl="{level}" w:tplc="{code}"><w:start w:val="1"/><w:numFmt w:val="{format}"/><w:lvlText w:val="%{number}."/><w:lvlJc w:val="{jc}"/><w:pPr><w:ind w:left="{indent}" w:hanging="{hanging}"/></w:pPr></w:lvl>"#
            ));
        }
    }
    format!(
        r#"<w:abstractNum {W_DECL} w:abstractNumId="{abstract_id}"><w:nsid w:val="{nsid}"/><w:multiLevelType w:val="hybridMultilevel"/><w:tmpl w:val="{template}"/>{levels}</w:abstractNum>"#
    )
}

/// Insert a definition after the last one (they precede every instance; docx-agent's
/// `add_abstract`), a declaration in it that the part's root makes already left out (as
/// lxml moves a subtree).
pub(crate) fn add_abstract(sink: &mut EditSink<'_, '_>, main: &str, node: Arc<Element>) -> Result<()> {
    let part = ensure_part(sink, main)?;
    let root = root(sink, &part)?;
    let node = strip_redundant(&node, &mut root.namespaces().to_vec());
    let at = last_of(&root, "abstractNum")
        .map(|k| k + 1)
        .or_else(|| first_of(&root, "num"))
        .or_else(|| first_of(&root, "numIdMacAtCleanup"))
        .unwrap_or(root.children().len());
    sink.splice_children(
        &part,
        (&[], at, 0),
        vec![Node::Element(node)],
        vec![part_key(&part)],
        Vec::new(),
    )
}

/// A new list of Word's own `bullet` or `number` kind: a definition of its own and an
/// instance over it (docx-agent's `new_list`); its number.
///
/// # Errors
/// When the numbering part is not XML.
pub(crate) fn new_list(sink: &mut EditSink<'_, '_>, main: &str, kind: &str) -> Result<i64> {
    let nums = match part_of(sink, main)? {
        Some(part) => listed(root(sink, &part)?.as_ref(), "num", "numId").len(),
        None => 0,
    };
    let abstract_id = add_builtin(sink, main, kind, |id| format!("{kind}\0{id}\0{nums}"))?;
    add_num(sink, main, abstract_id, &[])
}

/// Word's own `bullet` or `number` definition added after the last one, numbered past
/// them, its `w:nsid` and `w:tmpl` drawn from `seed` of its number (docx-agent's
/// `builtin_abstract` and `add_abstract`); its number.
///
/// # Errors
/// When the numbering part is not XML.
pub(crate) fn add_builtin(
    sink: &mut EditSink<'_, '_>,
    main: &str,
    kind: &str,
    seed: impl FnOnce(i64) -> String,
) -> Result<i64> {
    let part = part_of(sink, main)?;
    let abstracts = match &part {
        Some(part) => listed(root(sink, part)?.as_ref(), "abstractNum", "abstractNumId"),
        None => Vec::new(),
    };
    let abstract_id = abstracts.iter().map(|(n, _)| *n).max().unwrap_or(-1) + 1;
    let seed = seed(abstract_id);
    let used = used_nsids(sink, part.as_deref())?;
    let xml = builtin_abstract(
        kind,
        abstract_id,
        &nsid(&format!("{seed}nsid"), &used),
        &nsid(&format!("{seed}tmpl"), &used),
    );
    add_abstract(sink, main, parsed(&xml)?)?;
    Ok(abstract_id)
}

/// A new instance over `abstract_id` after the last one, `overrides` as `(level, start)`
/// (docx-agent's `add_num`); its number.
///
/// # Errors
/// When the numbering part is not XML.
pub(crate) fn add_num(
    sink: &mut EditSink<'_, '_>,
    main: &str,
    abstract_id: i64,
    overrides: &[(i64, i64)],
) -> Result<i64> {
    let mut sorted = overrides.to_vec();
    sorted.sort_unstable();
    let inner: String = sorted
        .iter()
        .map(|(level, start)| {
            format!(r#"<w:lvlOverride w:ilvl="{level}"><w:startOverride w:val="{start}"/></w:lvlOverride>"#)
        })
        .collect();
    insert_num(sink, main, abstract_id, &inner, Vec::new())
}

/// A new instance over `abstract_id` after the last one, holding `overrides` (another
/// list's `w:lvlOverride` elements, copied as lxml copies them: a declaration the part's
/// root makes already left out); its number (docx-agent's `add_num`, the overrides then
/// appended).
///
/// # Errors
/// When the numbering part is not XML.
pub(crate) fn add_num_copying(
    sink: &mut EditSink<'_, '_>,
    main: &str,
    abstract_id: i64,
    overrides: Vec<Arc<Element>>,
) -> Result<i64> {
    insert_num(sink, main, abstract_id, "", overrides)
}

/// A `w:num` numbered past the last, over `abstract_id`, its children `inner` (XML) and
/// then `copied`, after the last instance.
fn insert_num(
    sink: &mut EditSink<'_, '_>,
    main: &str,
    abstract_id: i64,
    inner: &str,
    copied: Vec<Arc<Element>>,
) -> Result<i64> {
    let part = ensure_part(sink, main)?;
    let root = root(sink, &part)?;
    let num_id = listed(&root, "num", "numId").iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
    let xml = format!(r#"<w:num {W_DECL} w:numId="{num_id}"><w:abstractNumId w:val="{abstract_id}"/>{inner}</w:num>"#);
    let mut num = parsed(&xml)?;
    if !copied.is_empty() {
        let mut scope = root.namespaces().to_vec();
        let mut children = num.children().to_vec();
        children.extend(copied.iter().map(|o| Node::Element(strip_redundant(o, &mut scope))));
        num = Arc::new(num.with_children(ooxml_xml::Children::from_vec(children)));
    }
    let at = last_of(&root, "num")
        .map(|k| k + 1)
        .or_else(|| first_of(&root, "numIdMacAtCleanup"))
        .unwrap_or(root.children().len());
    sink.splice_children(
        &part,
        (&[], at, 0),
        vec![Node::Element(num)],
        vec![part_key(&part)],
        Vec::new(),
    )?;
    Ok(num_id)
}

/// Replace the root's child at `k` by what `edit` makes of it in a [`Work`].
fn edit_child(sink: &mut EditSink<'_, '_>, part: &str, k: usize, edit: impl FnOnce(&mut Work, Id)) -> Result<()> {
    let root = root(sink, part)?;
    let element = child(&root, k).ok_or_else(|| EditError::no_element(part.to_owned()))?;
    let path = [u32::try_from(k).map_err(|_| EditError::no_element(part.to_owned()))?];
    let mut work = Work::of(element, scope_above(&root, &path));
    let top = work.root();
    edit(&mut work, top);
    let new = work.build().ok_or_else(|| EditError::unbuilt(part.to_owned()))?;
    if *new != **element {
        sink.replace(part, &path, new, vec![part_key(part)])?;
    }
    Ok(())
}

/// The definition `abstract_id`'s place in the part.
pub(crate) fn abstract_place(root: &Element, abstract_id: i64) -> Option<usize> {
    listed(root, "abstractNum", "abstractNumId")
        .into_iter()
        .find(|(n, _)| *n == abstract_id)
        .map(|(_, k)| k)
}

/// Change the `w:lvl` `level` of definition `abstract_id` by `edit`; `false` when the
/// definition has no such level.
///
/// # Errors
/// When the numbering part is not XML.
pub(crate) fn edit_level(
    sink: &mut EditSink<'_, '_>,
    part: &str,
    (abstract_id, level): (i64, i64),
    edit: impl FnOnce(&mut Work, Id),
) -> Result<bool> {
    let root = root(sink, part)?;
    let k = abstract_place(&root, abstract_id).ok_or_else(|| EditError::no_element(part.to_owned()))?;
    let has = child(&root, k).is_some_and(|a| {
        a.child_elements()
            .any(|l| l.name().is(Some(W), "lvl") && l.attribute(Some(W), "ilvl").and_then(py_int) == Some(level))
    });
    if !has {
        return Ok(false);
    }
    edit_child(sink, part, k, |work, top| {
        let lvl = work
            .children(top)
            .iter()
            .copied()
            .find(|c| work.is(*c, W, "lvl") && work.get(*c, Some(W), "ilvl").and_then(py_int) == Some(level));
        if let Some(lvl) = lvl {
            edit(work, lvl);
        }
    })?;
    Ok(true)
}

/// Point instance `num_id` at definition `abstract_id`.
///
/// # Errors
/// When the numbering part is not XML.
pub(crate) fn set_abstract_of(sink: &mut EditSink<'_, '_>, part: &str, num_id: i64, abstract_id: i64) -> Result<()> {
    let root = root(sink, part)?;
    let Some((_, k)) = listed(&root, "num", "numId").into_iter().find(|(n, _)| *n == num_id) else {
        return Ok(());
    };
    edit_child(sink, part, k, |work, top| {
        if let Some(reference) = work.find(top, W, "abstractNumId") {
            let val = work.qname(W, "val", "w");
            work.set(reference, val, &abstract_id.to_string());
        }
    })
}

/// A copy of definition `abstract_id` with a number and a `w:nsid` of its own, after the
/// last definition (docx-agent's `copy_abstract`); its number.
///
/// # Errors
/// When the numbering part is not XML.
pub(crate) fn copy_abstract(sink: &mut EditSink<'_, '_>, main: &str, part: &str, abstract_id: i64) -> Result<i64> {
    let root = root(sink, part)?;
    let abstracts = listed(&root, "abstractNum", "abstractNumId");
    let k = abstract_place(&root, abstract_id).ok_or_else(|| EditError::no_element(part.to_owned()))?;
    let original = child(&root, k).ok_or_else(|| EditError::no_element(part.to_owned()))?;
    let new_id = abstracts.iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
    let value = nsid(
        &format!("copy\0{abstract_id}\0{new_id}"),
        &used_nsids(sink, Some(part))?,
    );
    let mut work = Work::of(original, Vec::new());
    let top = work.root();
    let id_name = work.qname(W, "abstractNumId", "w");
    work.set(top, id_name, &new_id.to_string());
    let val = work.qname(W, "val", "w");
    let found = work.find(top, W, "nsid");
    let node = found.unwrap_or_else(|| {
        let made = work.new_element(work.element_name(top, W, "nsid", "w"));
        work.insert(top, 0, made);
        made
    });
    work.set(node, val, &value);
    let copy = work.build().ok_or_else(|| EditError::unbuilt(part.to_owned()))?;
    add_abstract(sink, main, copy)?;
    Ok(new_id)
}

/// Every `numId` a paragraph of any story, or a style, names (docx-agent's
/// `referenced_nums`): read from every story part and the styles.
pub(crate) fn referenced(docx: &Docx) -> BTreeSet<i64> {
    fn walk(element: &Element, out: &mut BTreeSet<i64>) {
        for c in element.child_elements() {
            if c.name().is(Some(W), "numPr") {
                for r in c.child_elements().filter(|r| r.name().is(Some(W), "numId")) {
                    if let Some(v) = r.attribute(Some(W), "val").and_then(py_int).filter(|v| *v != 0) {
                        out.insert(v);
                    }
                }
            }
            walk(c, out);
        }
    }
    let mut out = BTreeSet::new();
    for story in &docx.parts {
        walk(story.root(), &mut out);
    }
    for style in docx.styles() {
        walk(style, &mut out);
    }
    out
}

/// Remove the instances among `candidates` nothing names any more, then the definitions
/// only they used and that no list style links (docx-agent's `reap`); what went
/// (`num:3`, `abstractNum:2`).
///
/// # Errors
/// When a part is not XML.
pub(crate) fn reap(sink: &mut EditSink<'_, '_>, main: &str, candidates: &BTreeSet<i64>) -> Result<Vec<String>> {
    let Some(part) = part_of(sink, main)? else {
        return Ok(Vec::new());
    };
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let used = referenced(sink.docx()?.as_ref());
    let mut removed = Vec::new();
    let mut touched = BTreeSet::new();
    for num_id in candidates {
        let root = root(sink, &part)?;
        let nums = listed(&root, "num", "numId");
        let Some((_, k)) = nums.iter().find(|(n, _)| n == num_id).copied() else {
            continue;
        };
        if used.contains(num_id) {
            continue;
        }
        if let Some(a) = child(&root, k).and_then(|n| n.find(Some(W), "abstractNumId")) {
            if let Some(a) = a.attribute(Some(W), "val").and_then(py_int) {
                touched.insert(a);
            }
        }
        sink.splice_children(&part, (&[], k, 1), Vec::new(), vec![part_key(&part)], Vec::new())?;
        removed.push(format!("num:{num_id}"));
    }
    if removed.is_empty() {
        return Ok(removed);
    }
    for abstract_id in touched {
        let root = root(sink, &part)?;
        let still: HashSet<i64> = listed(&root, "num", "numId")
            .iter()
            .filter_map(|(_, k)| {
                child(&root, *k)?
                    .find(Some(W), "abstractNumId")?
                    .attribute(Some(W), "val")
                    .and_then(py_int)
            })
            .collect();
        let Some(k) = abstract_place(&root, abstract_id) else {
            continue;
        };
        let linked = child(&root, k).is_some_and(|a| a.find(Some(W), "styleLink").is_some());
        if !still.contains(&abstract_id) && !linked {
            sink.splice_children(&part, (&[], k, 1), Vec::new(), vec![part_key(&part)], Vec::new())?;
            removed.push(format!("abstractNum:{abstract_id}"));
        }
    }
    Ok(removed)
}

/// A new instance over `abstract_id` (the definition of `num_id`, the style's instance)
/// that restarts `level` at `start`, for an item its style (`style_id`) numbers
/// (docx-agent's `add_restart`): where the style's indent differs from the level's, the
/// whole level in the override, starting at `start`, with the style's indent; its number.
///
/// # Errors
/// When the numbering part is not XML.
pub(crate) fn add_restart(
    sink: &mut EditSink<'_, '_>,
    main: &str,
    (abstract_id, num_id, level): (i64, i64, i64),
    start: i64,
    style_id: Option<&str>,
) -> Result<i64> {
    let new_id = add_num(sink, main, abstract_id, &[(level, start)])?;
    let docx = sink.docx()?;
    let Some(style_ind) = super::read::style_indent(&docx, style_id) else {
        return Ok(new_id);
    };
    if super::read::indent_note(&docx, (num_id, level), Some(&style_ind)).is_none() {
        return Ok(new_id);
    }
    let part = ensure_part(sink, main)?;
    let root = root(sink, &part)?;
    let lists = ooxml_word_model::docx::numbering::Lists::read(&docx);
    let Some(canonical) = lists.canonical(Some(abstract_id)) else {
        return Ok(new_id);
    };
    let source = abstract_place(&root, canonical)
        .and_then(|k| child(&root, k))
        .and_then(|a| {
            a.child_elements()
                .find(|l| l.name().is(Some(W), "lvl") && l.attribute(Some(W), "ilvl").and_then(py_int) == Some(level))
                .cloned()
        });
    let Some(source) = source else {
        return Ok(new_id);
    };
    let shared = sink.doc().shared_handle();
    let orders = shared.facts.orders();
    let mut work = Work::of(&source, Vec::new());
    let lvl = work.root();
    let val = work.qname(W, "val", "w");
    let first = work.find(lvl, W, "start").unwrap_or_else(|| {
        let made = work.new_element(work.element_name(lvl, W, "start", "w"));
        work.insert(lvl, 0, made);
        made
    });
    work.set(first, val, &start.to_string());
    let properties = work.find(lvl, W, "pPr").unwrap_or_else(|| {
        let made = work.new_element(work.element_name(lvl, W, "pPr", "w"));
        crate::marks::insert_in_order(&mut work, lvl, made, orders);
        made
    });
    if let Some(old) = work.find(properties, W, "ind") {
        work.remove(old);
    }
    if let Some(ind) = work.adopt(&style_ind) {
        crate::marks::insert_in_order(&mut work, properties, ind, orders);
    }
    let copied = work.build().ok_or_else(|| EditError::unbuilt(part.clone()))?;
    let Some((_, k)) = listed(&root, "num", "numId").into_iter().find(|(n, _)| *n == new_id) else {
        return Ok(new_id);
    };
    edit_child(sink, &part, k, |work, num| {
        let found =
            work.children(num).iter().copied().find(|o| {
                work.is(*o, W, "lvlOverride") && work.get(*o, Some(W), "ilvl").and_then(py_int) == Some(level)
            });
        if let (Some(over), Some(node)) = (found, work.adopt(&copied)) {
            work.append(over, node);
        }
    })?;
    Ok(new_id)
}
