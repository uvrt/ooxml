//! Owns: writing one planned range across paragraphs ([`write_across`]): the stretch of
//! its parent's children from the first paragraph to the last copied into a work, the
//! range replaced there by the policy ([`crate::across`]), and the result spliced back in
//! place of the stretch -- its revision ids, and the new text ids docx-agent gives the
//! paragraphs left (untracked the last; tracked the first and the last), drawn where the
//! change lands.
//!
//! Must not: write but through the sink, or touch children outside the stretch.

use crate::doc::{Draw, DrawFrom, Key};
use crate::names::W14;
use crate::ranges::{stretch, Plan, Written};
use crate::set_text::{block_key, paragraph_has_annotations};
use crate::sink::{annotation_draws, EditSink, Keep};
use crate::stamping::w14_prefix;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::{Element, Node, QName};
use std::sync::Arc;

/// The place among the nodes [`Work::build_content`] makes of `child`, a child of `root`.
fn content_index(work: &Work, root: Id, child: Id) -> Option<u32> {
    let mut at = usize::from(!work.text(root).is_empty());
    for &c in work.children(root) {
        if c == child {
            return u32::try_from(at).ok();
        }
        at += 1 + usize::from(!work.tail(c).is_empty());
    }
    None
}

/// The keys the stretch at `paths` of `root` is written under, made
/// `nodes`: its paragraphs' blocks, and the annotation ids in use when it held revision
/// records or does now (the ids a tracked edit numbers from, as `set_text` declares them).
fn stretch_keys(docx: &Docx, part: &str, (paths, root): (&[Box<[u32]>], &Arc<Element>), nodes: &[Node]) -> Vec<Key> {
    let mut keys: Vec<Key> = paths
        .iter()
        .filter_map(|p| docx.part(part).and_then(|s| s.ids.paragraph_at(p)))
        .map(|p| block_key(part, &p.id))
        .collect();
    let annotated = |n: &Node| matches!(n, Node::Element(e) if paragraph_has_annotations(e));
    if nodes.iter().any(annotated)
        || paths
            .iter()
            .filter_map(|p| at_path(root, p))
            .any(|e| paragraph_has_annotations(e))
    {
        keys.push(Key::Annotations);
    }
    keys
}

/// Replace `plan`'s range across paragraphs and splice the stretch back.
///
/// # Errors
/// When the stretch is not where the plan found it, or the range cannot be replaced.
pub(crate) fn write_across(sink: &mut EditSink<'_, '_>, plan: &Plan, keep: Keep) -> Result<Option<Written>> {
    let Some(across) = plan.across.as_ref() else {
        return Ok(None);
    };
    let part = plan.found.part.clone();
    let tree = sink
        .doc()
        .tree(&part)?
        .ok_or_else(|| EditError::no_part(part.clone()))?;
    let path_error = || EditError::no_element(part.clone());
    let (mut work, paragraphs) = stretch(tree.root(), &across.paths).ok_or_else(path_error)?;
    let (&first, &last) = paragraphs.first().zip(paragraphs.last()).ok_or_else(path_error)?;
    sink.load_written(&part, None, &mut work);
    let wrote = sink.replace_across(&mut work, &paragraphs, (plan.start, plan.end), &plan.text, keep)?;
    sink.keep_written(&part, None, &work);
    let root = work.root();
    let nodes = work.build_content(root).ok_or_else(path_error)?;
    let mut draws = Vec::new();
    if let Some(numbered) = wrote.annotations {
        for (k, node) in nodes.iter().enumerate() {
            if let (Node::Element(e), Ok(k)) = (node, u32::try_from(k)) {
                draws.extend(annotation_draws(e, numbered, k));
            }
        }
    }
    // the paragraphs left get new text ids: untracked the last, tracked the first too
    let renewed: Vec<Id> = if wrote.annotations.is_some() {
        vec![first, last]
    } else {
        vec![last]
    };
    let (first_path, last_path) = (
        across.paths.first().ok_or_else(path_error)?,
        across.paths.last().ok_or_else(path_error)?,
    );
    let (&from, parent) = first_path.split_last().ok_or_else(path_error)?;
    let &to = last_path.last().ok_or_else(path_error)?;
    let prefix = w14_prefix(tree.root(), first_path);
    for paragraph in renewed {
        let Some(k) = content_index(&work, root, paragraph) else {
            continue; // removed (one's own insertion, joined to the next)
        };
        let stamped =
            matches!(nodes.get(k as usize), Some(Node::Element(e)) if e.attribute(Some(W14), "paraId").is_some());
        if stamped {
            draws.push(Draw {
                from: DrawFrom::Seed(Arc::from(format!("{part}\0text"))),
                node: k,
                at: Vec::new(),
                attribute: Arc::new(QName::new(Some(&prefix), "textId", Some(W14))),
            });
        }
    }
    let docx = sink.docx()?;
    let keys = stretch_keys(&docx, &part, (&across.paths, tree.root()), &nodes);
    // docx-agent: aliases naming a paragraph the edit joined away go with it
    let gone: Vec<String> = paragraphs
        .iter()
        .zip(&across.paths)
        .filter(|(p, _)| content_index(&work, root, **p).is_none())
        .filter_map(|(_, path)| docx.part(&part).and_then(|s| s.ids.paragraph_at(path)))
        .map(|p| p.id)
        .collect();
    sink.splice_children(
        &part,
        (parent, from as usize, (to - from) as usize + 1),
        nodes,
        keys,
        draws,
    )?;
    sink.forget_alias_targets(&gone)?;
    Ok(Some(Written {
        part,
        released: wrote.released,
        dated: wrote.dated,
        gave: wrote.annotations.is_some_and(|n| n.to > n.from),
    }))
}
