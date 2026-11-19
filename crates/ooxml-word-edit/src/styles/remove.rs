//! Owns: a style taken out of the sheet as docx-agent's `Styles` takes it (`edit/styles.py`):
//! where content uses a style (`_content_references`: paragraphs, runs, tables and list
//! levels, every part but the sheet), `remove` (refused while used unless a replacement is
//! given: every use then names it, styles based on it take its base, a next style or link
//! to it goes; never a default style), and `purge_unused` (the custom styles nothing uses,
//! a linked pair together, until none is left).
//!
//! Must not: write a story's block but under its key: a use repointed is its block's
//! change (the innermost paragraph or table holding it).

use super::read::{content_parts, CONTENT_REFERENCES};
use super::sheet::{self, on, style_references, Sheet, REL_STYLES};
use super::{not_found, StyleStep};
use crate::builtin;
use crate::doc::Key;
use crate::names::W;
use crate::review::space::Space;
use crate::set_text::block_key;
use crate::sink::EditSink;
use crate::work::Work;
use crate::{EditError, Result};
use ooxml_base::pyfmt::repr_str;
use ooxml_xml::{Element, Node};
use std::sync::Arc;

fn refused(message: impl Into<String>) -> EditError {
    EditError::Refused(message.into())
}

/// The paths (child indexes) of the elements under `element` naming style `id`.
fn paths(element: &Element, id: &str, at: &mut Vec<u32>, out: &mut Vec<Vec<u32>>) {
    if element.name().namespace() == Some(W)
        && CONTENT_REFERENCES.contains(&element.name().local())
        && element.attribute(Some(W), "val") == Some(id)
    {
        out.push(at.clone());
    }
    for (k, child) in element.children().iter().enumerate() {
        if let Node::Element(child) = child {
            at.push(u32::try_from(k).unwrap_or(u32::MAX));
            paths(child, id, at, out);
            at.pop();
        }
    }
}

fn count(work: &Work, id: &str) -> usize {
    work.iter(work.root(), W, CONTENT_REFERENCES)
        .into_iter()
        .filter(|n| work.get(*n, Some(W), "val") == Some(id))
        .count()
}

/// How many times content names style `id` now (`len(_content_references(id))`): the
/// edit's copies where it holds one.
///
/// # Errors
/// When a part is not XML.
pub(crate) fn uses(sink: &mut EditSink<'_, '_>, space: &Space, id: &str) -> Result<usize> {
    let docx = sink.docx()?;
    let styles = ooxml_word_model::parts::related(sink.doc().package(), &docx.main, REL_STYLES)?;
    let mut total = 0;
    for part in content_parts(&docx, styles.as_deref()) {
        if let Some(work) = space.get(&part) {
            total += count(work, id);
        } else if let Some(tree) = sink.doc().tree(&part).ok().flatten() {
            let mut found = Vec::new();
            paths(tree.root(), id, &mut Vec::new(), &mut found);
            total += found.len();
        }
    }
    Ok(total)
}

/// `element` with `w:val` of the element at each of `at` (relative paths) set to `value`.
fn with_vals(element: &Arc<Element>, at: &[&[u32]], value: &str) -> Arc<Element> {
    let mut children: Vec<Node> = element.children().iter().cloned().collect();
    for (k, child) in children.iter_mut().enumerate() {
        let below: Vec<&[u32]> = at
            .iter()
            .filter_map(|p| p.split_first())
            .filter(|(first, _)| **first as usize == k)
            .map(|(_, rest)| rest)
            .collect();
        if let (false, Node::Element(e)) = (below.is_empty(), &*child) {
            *child = Node::Element(with_vals(e, &below, value));
        }
    }
    let mut out = element.with_children(ooxml_xml::Children::from_vec(children));
    if at.iter().any(|p| p.is_empty()) {
        let prefix = element.name().prefix().map(str::to_owned);
        let name: ooxml_xml::Name = Arc::new(ooxml_xml::QName::new(prefix.as_deref(), "val", Some(W)));
        out = out.with_head(crate::stamping::with_attribute(element.head(), &name, value));
    }
    Arc::new(out)
}

/// Every use of style `id` in the content pointed at `target`, each block's change under
/// its key.
fn repoint(sink: &mut EditSink<'_, '_>, id: &str, target: &str) -> Result<()> {
    let docx = sink.docx()?;
    let styles = ooxml_word_model::parts::related(sink.doc().package(), &docx.main, REL_STYLES)?;
    for part in content_parts(&docx, styles.as_deref()) {
        let Some(tree) = sink.doc().tree(&part).ok().flatten() else {
            continue;
        };
        let mut found = Vec::new();
        paths(tree.root(), id, &mut Vec::new(), &mut found);
        // each use under the innermost block holding it (a story's), else itself
        let story = docx.part(&part);
        let mut groups: Vec<(Vec<u32>, Key, Vec<Vec<u32>>)> = Vec::new();
        for path in found {
            let block = story.and_then(|s| {
                (1..=path.len()).rev().find_map(|n| {
                    let prefix = path.get(..n)?;
                    let entry = s.ids.at(prefix)?;
                    s.ids.id_of(entry).map(|i| (prefix.to_vec(), block_key(&part, &i)))
                })
            });
            let (at, key) = block.unwrap_or_else(|| (path.clone(), Key::Part(Arc::from(part.as_str()))));
            let rest = path.get(at.len()..).unwrap_or(&[]).to_vec();
            match groups.iter_mut().find(|(p, _, _)| *p == at) {
                Some(group) => group.2.push(rest),
                None => groups.push((at, key, vec![rest])),
            }
        }
        for (at, key, rests) in groups {
            let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
            let element =
                ooxml_word_model::docx::at_path(tree.root(), &at).ok_or_else(|| EditError::Path(part.clone()))?;
            let rests: Vec<&[u32]> = rests.iter().map(Vec::as_slice).collect();
            let new = with_vals(element, &rests, target);
            if at.is_empty() {
                sink.put(&part, tree.with_root(new))?;
            } else {
                sink.replace(&part, &at, new, vec![key])?;
            }
        }
    }
    Ok(())
}

/// Whether `name` is one of the sheet's latent (built-in, undefined) styles (`latent`).
fn latent(root: &Element, name: &str) -> bool {
    let wanted = ooxml_word_model::docx::casefold(name);
    root.find(Some(W), "latentStyles").is_some_and(|l| {
        l.child_elements().any(|e| {
            e.name().is(Some(W), "lsdException")
                && ooxml_word_model::docx::casefold(e.attribute(Some(W), "name").unwrap_or("")) == wanted
        })
    })
}

/// How many times content names each style (one reading of every part).
fn content_counts(sink: &mut EditSink<'_, '_>) -> Result<std::collections::HashMap<String, usize>> {
    fn walk(element: &Element, out: &mut std::collections::HashMap<String, usize>) {
        if element.name().namespace() == Some(W) && CONTENT_REFERENCES.contains(&element.name().local()) {
            if let Some(val) = element.attribute(Some(W), "val") {
                *out.entry(val.to_owned()).or_insert(0) += 1;
            }
        }
        for child in element.child_elements() {
            walk(child, out);
        }
    }
    let docx = sink.docx()?;
    let styles = ooxml_word_model::parts::related(sink.doc().package(), &docx.main, REL_STYLES)?;
    let mut out = std::collections::HashMap::new();
    for part in content_parts(&docx, styles.as_deref()) {
        if let Some(tree) = sink.doc().tree(&part).ok().flatten() {
            walk(tree.root(), &mut out);
        }
    }
    Ok(out)
}

/// `remove` (docx-agent's `Styles.remove`).
pub(crate) fn remove(
    sink: &mut EditSink<'_, '_>,
    space: &mut Space,
    name: &str,
    kind: Option<&str>,
    replacement: Option<&str>,
) -> Result<StyleStep> {
    sink.read_everything();
    let kind_word = kind.map(|k| format!("{k} ")).unwrap_or_default();
    let missing = || not_found(format!("the document has no {kind_word}style {}", repr_str(name)));
    let docx = sink.docx()?;
    let style = sheet::find_in(&docx, name, kind).ok_or_else(missing)?;
    let id = ooxml_word_model::docx::style_id(&style).to_owned();
    let style_kind = ooxml_word_model::docx::style_kind(&style).to_owned();
    let style_name = ooxml_word_model::docx::style_name(&style).map(str::to_owned);
    let quoted = style_name.as_deref().map_or_else(|| "None".to_owned(), repr_str);
    if on(Some(style.attribute(Some(W), "default").unwrap_or("0"))) {
        return Err(refused(format!(
            "{quoted} is the document's default {style_kind} style: it cannot be removed"
        )));
    }
    if let Some(replacement) = replacement {
        let found = sheet::find_in(&docx, replacement, Some(&style_kind));
        let latent_named = {
            let styles = ooxml_word_model::parts::related(sink.doc().package(), &docx.main, REL_STYLES)?;
            let tree = match &styles {
                Some(p) => sink.doc().tree(p)?,
                None => None,
            };
            tree.is_some_and(|t| latent(t.root(), replacement))
        };
        if found.is_none()
            && sheet::find_in(&docx, replacement, None).is_none()
            && !latent_named
            && builtin::resolve(&docx, replacement, &style_kind).is_err()
        {
            return Err(not_found(format!(
                "the document has no {style_kind} style {}",
                repr_str(replacement)
            )));
        }
        if found.is_some_and(|f| ooxml_word_model::docx::style_id(&f) == id) {
            return Err(refused("a style cannot replace itself"));
        }
    }
    let used = uses(sink, space, &id)?;
    if used > 0 && replacement.is_none() {
        return Err(refused(format!(
            "{} is used {used} time(s): give replacement= (a {style_kind} style) to point those at",
            repr_str(name)
        )));
    }
    let target = replacement
        .map(|r| {
            super::add::resolve(sink, r, &style_kind)?;
            builtin::ensure(sink, r, &style_kind)
        })
        .transpose()?;
    if let Some(target) = &target {
        repoint(sink, &id, target)?;
    }
    let sheet = Sheet::open(sink, space)?.ok_or_else(missing)?;
    let work = sheet.work(space)?;
    let node = sheet::by_id(work, sheet.root, &id).ok_or_else(missing)?;
    let based_on = sheet::child_val(work, node, "basedOn").map(str::to_owned);
    for reference in style_references(work, sheet.root, &id) {
        let is = |local: &str| work.is(reference, W, local);
        if is("basedOn") && based_on.is_some() {
            crate::review::xml::set_attr(work, reference, "val", based_on.as_deref().unwrap_or(""));
        } else if (is("pStyle") || is("rStyle") || is("tblStyle")) && target.is_some() {
            crate::review::xml::set_attr(work, reference, "val", target.as_deref().unwrap_or(""));
        } else {
            work.remove(reference);
        }
    }
    work.remove(node);
    let mut step = StyleStep::of(None);
    step.removed = vec![format!("style:{id}")];
    if used > 0 {
        step.warnings = vec![format!(
            "{used} use(s) now name {}",
            replacement.map_or_else(|| "None".to_owned(), repr_str)
        )];
    }
    Ok(step)
}

/// `purge_unused` (docx-agent's `Styles.purge_unused`, custom styles only).
pub(crate) fn purge_unused(sink: &mut EditSink<'_, '_>, space: &mut Space) -> Result<StyleStep> {
    sink.read_everything();
    let Some(sheet) = Sheet::open(sink, space)? else {
        return Ok(StyleStep::default());
    };
    // nothing a purge does changes the content: its uses are read once
    let counts = content_counts(sink)?;
    let used_by = |id: &str| counts.get(id).copied().unwrap_or(0) > 0;
    let mut removed: Vec<String> = Vec::new();
    let purgeable = |work: &Work, node| {
        !on(Some(work.get(node, Some(W), "default").unwrap_or("0")))
            && on(Some(work.get(node, Some(W), "customStyle").unwrap_or("0")))
    };
    let mut changed = true;
    while changed {
        changed = false;
        let nodes: Vec<_> = sheet::entries(sheet.view(space)?, sheet.root);
        for style in nodes {
            if !purgeable(sheet.view(space)?, style.node) || used_by(&style.id) {
                continue;
            }
            let work = sheet.view(space)?;
            if work.parent(style.node).is_none() {
                continue;
            }
            let references = style_references(work, sheet.root, &style.id);
            if references.iter().any(|r| !work.is(*r, W, "link")) {
                continue;
            }
            // a linked pair goes together: kept while the other one is used
            let mut kept = false;
            for reference in &references {
                let Some(owner) = work.parent(*reference) else { continue };
                let other = sheet::entry(work, owner);
                let used = used_by(&other.id)
                    || style_references(work, sheet.root, &other.id)
                        .iter()
                        .any(|r| !work.is(*r, W, "link"))
                    || on(Some(work.get(owner, Some(W), "default").unwrap_or("0")));
                if used || !purgeable(work, owner) {
                    kept = true;
                    break;
                }
            }
            if kept {
                continue;
            }
            let work = sheet.work(space)?;
            for reference in style_references(work, sheet.root, &style.id) {
                work.remove(reference);
            }
            work.remove(style.node);
            removed.push(format!("style:{}", style.id));
            changed = true;
        }
    }
    Ok(StyleStep {
        id: None,
        changed: !removed.is_empty(),
        removed,
        ..StyleStep::default()
    })
}
