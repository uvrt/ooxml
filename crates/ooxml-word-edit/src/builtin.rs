//! Owns: a style named for an edit, as docx-agent's `Styles.resolve` and `_ensure` find or
//! add it ([`resolve`], [`ensure`]): the document's own by name, alias or id; else Word's
//! built-in definition written as Word writes it on first use (`builtin_styles.write`),
//! from the measured table (`data/builtin-styles.tsv`, generated from docx-agent's
//! definitions) -- with what it points at that the document lacks, its references given
//! the document's ids, and a list style's numbering.
//!
//! Must not: write but through the sink, or hold state: the table is parsed once into an
//! immutable value.

use crate::doc::Key;
use crate::names::W;
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_word_model::docx::{casefold, Docx};
use ooxml_xml::{Element, Node};
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

/// The relationship type of the styles part.
const REL_STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
/// The styles part's content type.
const STYLES_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml";
/// The kinds a style is of.
const KINDS: &[&str] = &["paragraph", "character", "table", "numbering"];

/// One measured definition.
#[derive(Debug)]
struct Builtin {
    name: String,
    kind: String,
    /// The styles its `w:basedOn`, `w:next` and `w:link` point at, by name.
    refs: [(&'static str, String); 3],
    xml: String,
    numbering: String,
}

/// The measured definitions, in the source's order.
fn table() -> &'static [Builtin] {
    static TABLE: OnceLock<Vec<Builtin>> = OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("../data/builtin-styles.tsv")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .filter_map(|line| {
                let cells: Vec<&str> = line.split('\t').collect();
                let [name, kind, based_on, next, link, xml, numbering] = cells.as_slice() else {
                    return None;
                };
                Some(Builtin {
                    name: (*name).to_owned(),
                    kind: (*kind).to_owned(),
                    refs: [
                        ("basedOn", (*based_on).to_owned()),
                        ("next", (*next).to_owned()),
                        ("link", (*link).to_owned()),
                    ],
                    xml: (*xml).to_owned(),
                    numbering: (*numbering).to_owned(),
                })
            })
            .collect()
    })
}

/// The definition `name` names (exactly, then in any case).
fn lookup(name: &str) -> Option<&'static Builtin> {
    let table = table();
    table.iter().find(|b| b.name == name).or_else(|| {
        let wanted = casefold(name);
        table.iter().find(|b| casefold(&b.name) == wanted)
    })
}

/// English Word's id for a built-in name: its words run together, each capitalised, but
/// for the names whose id is not that.
fn english_id(name: &str) -> String {
    match name {
        "annotation text" => return "CommentText".to_owned(),
        "Annotation Text Char" => return "CommentTextChar".to_owned(),
        "annotation reference" => return "CommentReference".to_owned(),
        "annotation subject" => return "CommentSubject".to_owned(),
        "Annotation Subject Char" => return "CommentSubjectChar".to_owned(),
        _ => {}
    }
    if let Some(n) = name
        .strip_prefix("toc ")
        .filter(|n| n.len() == 1 && n.chars().all(|c| ('1'..='9').contains(&c)))
    {
        return format!("TOC{n}");
    }
    name.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect()
}

/// The style of any kind `name` names in the document, if there is one.
fn find_any<'d>(docx: &'d Docx, name: &str) -> Option<&'d Arc<Element>> {
    KINDS.iter().find_map(|kind| docx.find_style(name, kind))
}

/// A style's id.
fn style_id(style: &Element) -> &str {
    style.attribute(Some(W), "styleId").unwrap_or("")
}

/// What adding the built-in `name` writes: (canonical name, new id, definition) for it and
/// each style it points at the document lacks; `None` when Word has no built-in `kind`
/// style by that name (docx-agent's `plan`).
fn plan(docx: &Docx, name: &str, kind: &str) -> Option<Vec<(String, String, &'static Builtin)>> {
    let found = lookup(name).filter(|b| b.kind == kind)?;
    let mut taken: HashSet<String> = docx.styles().iter().map(|s| casefold(style_id(s))).collect();
    let mut out: Vec<(String, String, &'static Builtin)> = Vec::new();
    let mut pending = vec![found.name.clone()];
    while !pending.is_empty() {
        let current = pending.remove(0);
        if out.iter().any(|(planned, _, _)| *planned == current) || find_any(docx, &current).is_some() {
            continue;
        }
        let Some(entry) = lookup(&current) else { continue };
        let base = english_id(&entry.name);
        let mut id = base.clone();
        let mut number = 1;
        while taken.contains(&casefold(&id)) {
            number += 1;
            id = format!("{base}{number}");
        }
        taken.insert(casefold(&id));
        out.push((entry.name.clone(), id, entry));
        pending.extend(entry.refs.iter().filter(|(_, r)| !r.is_empty()).map(|(_, r)| r.clone()));
    }
    Some(out)
}

/// The id of the `kind` style `name` names, the document's own or a built-in one adding it
/// would write (docx-agent's `Styles.resolve`); nothing changes.
///
/// # Errors
/// [`EditError::Refused`] for a style of another kind, or a name neither the document nor
/// Word knows.
pub(crate) fn resolve(docx: &Docx, name: &str, kind: &str) -> Result<String> {
    if let Some(found) = docx.find_style(name, kind) {
        return Ok(style_id(found).to_owned());
    }
    if let Some(other) = find_any(docx, name) {
        let theirs = ooxml_word_model::docx::style_kind(other);
        return Err(EditError::Refused(format!(
            "'{name}' is a {theirs} style, not a {kind} style"
        )));
    }
    plan(docx, name, kind)
        .and_then(|p| p.first().map(|(_, id, _)| id.clone()))
        .ok_or_else(|| {
            EditError::Refused(format!(
                "the document has no {kind} style '{name}', and Word has no built-in one by that name"
            ))
        })
}

/// `element` with its child `w:{local}` given `value` (removed for `None`).
fn with_reference(element: &Arc<Element>, local: &str, value: Option<&str>) -> Arc<Element> {
    let Some(k) = element
        .children()
        .iter()
        .position(|n| matches!(n, Node::Element(e) if e.name().is(Some(W), local)))
    else {
        return Arc::clone(element);
    };
    let replacement: Vec<Node> = match (value, element.children().get(k)) {
        (Some(value), Some(Node::Element(child))) => {
            let head = crate::stamping::with_attribute(child.head(), &w_val(child), value);
            vec![Node::Element(Arc::new(Element::from_head(
                Arc::new(head),
                child.children().clone(),
            )))]
        }
        _ => Vec::new(),
    };
    match element.children().splice(k..k + 1, replacement) {
        Some(children) => Arc::new(element.with_children(children)),
        None => Arc::clone(element),
    }
}

/// The name `w:val` has on `element` (its prefix as the element's).
fn w_val(element: &Element) -> ooxml_xml::Name {
    let prefix = element.name().prefix().map(str::to_owned);
    Arc::new(ooxml_xml::QName::new(prefix.as_deref(), "val", Some(W)))
}

/// The id of the `kind` style `name` names, the built-in definition (and what it points at
/// that the document lacks) written into the styles part first when the document has none
/// (docx-agent's `Styles._ensure`).
///
/// # Errors
/// [`resolve`]'s refusals; when a part is not XML.
pub(crate) fn ensure(sink: &mut EditSink<'_, '_>, name: &str, kind: &str) -> Result<String> {
    let docx = sink.docx()?;
    let main = docx.main.clone();
    let found = ooxml_word_model::parts::related(sink.doc().package(), &main, REL_STYLES)?;
    // the edit depends on the style being there (or not): a style removed meanwhile is
    // a conflict, not a reference to nothing
    if let Some(part) = &found {
        sink.read(Key::part(part.as_str()));
    }
    if let Some(found) = docx.find_style(name, kind) {
        return Ok(style_id(found).to_owned());
    }
    resolve(&docx, name, kind)?;
    let planned = plan(&docx, name, kind).unwrap_or_default();
    let ids: Vec<(String, String)> = planned.iter().map(|(c, id, _)| (casefold(c), id.clone())).collect();
    let target = |reference: &str| -> Option<String> {
        let wanted = casefold(reference);
        ids.iter()
            .find(|(c, _)| *c == wanted)
            .map(|(_, id)| id.clone())
            .or_else(|| find_any(&docx, reference).map(|s| style_id(s).to_owned()))
    };
    let mut nodes = Vec::new();
    for (_, id, definition) in &planned {
        let mut node = crate::list_style::parsed(&definition.xml.replace("{id}", id))?;
        for (local, reference) in &definition.refs {
            node = with_reference(&node, local, target(reference).as_deref());
        }
        if !definition.numbering.is_empty() {
            let num = crate::list_style::add_list(sink, &main, id, &definition.numbering)?;
            node = crate::list_style::with_num(&node, &num);
        }
        nodes.push(Node::Element(node));
    }
    let part = match found {
        Some(part) => part,
        None => styles_part(sink)?,
    };
    let tree = sink
        .doc()
        .tree(&part)?
        .ok_or_else(|| EditError::no_part(part.clone()))?;
    let at = tree.root().children().len();
    sink.splice_children(&part, (&[], at, 0), nodes, vec![Key::part(part.as_str())], Vec::new())?;
    Ok(planned.first().map(|(_, id, _)| id.clone()).unwrap_or_default())
}

/// The styles part, made when the document has none (docx-agent's `_create_styles_part`,
/// when the first style is appended).
///
/// # Errors
/// When a part cannot be added.
pub(crate) fn styles_part(sink: &mut EditSink<'_, '_>) -> Result<String> {
    let docx = sink.docx()?;
    let main = docx.main.clone();
    if let Some(part) = ooxml_word_model::parts::related(sink.doc().package(), &main, REL_STYLES)? {
        return Ok(part);
    }
    sink.new_part(
        &main,
        ("word/styles.xml", "word/styles{n}.xml"),
        crate::new_part::word_root("styles"),
        STYLES_TYPE,
        REL_STYLES,
    )
}
