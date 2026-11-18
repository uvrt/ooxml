//! Owns: what the list edits read of a version, as docx-agent's `edit/numbering.py` reads
//! it -- the list a style numbers its paragraphs with ([`style_list`]), a style's indent
//! through its `basedOn` chain, the note on direct numbering that takes a level's indent
//! rather than the style's ([`indent_note`]), and a paragraph's place among its story's
//! paragraphs.
//!
//! Must not: write.

use crate::names::W;
use ooxml_word_model::docx::numbering::{membership, Lists, Membership};
use ooxml_word_model::docx::{find_path, Docx, StoryPart};
use ooxml_word_model::xmlread::py_int;
use ooxml_xml::Element;
use std::sync::Arc;

/// A paragraph by its place: its story part and its index among the part's paragraphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Place {
    /// The part.
    pub(crate) part: String,
    /// Its index among the part's paragraphs (document order).
    pub(crate) index: usize,
}

/// A paragraph found at its [`Place`].
pub(crate) struct At<'d> {
    /// Its story.
    pub(crate) story: &'d StoryPart,
    /// Its id.
    pub(crate) id: String,
    /// Its path in the part.
    pub(crate) path: Box<[u32]>,
    /// The paragraph.
    pub(crate) element: Arc<Element>,
}

/// The paragraph at `place` in `docx`.
pub(crate) fn at<'d>(docx: &'d Docx, place: &Place) -> Option<At<'d>> {
    let story = docx.part(&place.part)?;
    let paragraph = story.ids.paragraphs().get(place.index)?;
    Some(At {
        story,
        id: paragraph.id.clone(),
        path: paragraph.path.clone(),
        element: Arc::clone(&paragraph.element),
    })
}

/// The list the paragraph at `place` is in now.
pub(crate) fn list_at(docx: &Docx, place: &Place) -> Option<Membership> {
    let found = at(docx, place)?;
    membership(docx, &Lists::read(docx), &found.element)
}

/// The style a paragraph names (`None`: the default paragraph style).
pub(crate) fn paragraph_style_id(paragraph: &Element) -> Option<String> {
    find_path(paragraph, &["pPr", "pStyle"])
        .and_then(|s| s.attribute(Some(W), "val"))
        .map(str::to_owned)
}

/// The default paragraph style's id, when `style_id` is `None`.
fn style_or_default(docx: &Docx, style_id: Option<&str>) -> Option<String> {
    match style_id {
        Some(s) => Some(s.to_owned()),
        None => docx
            .styles()
            .iter()
            .find(|s| {
                s.attribute(Some(W), "type") == Some("paragraph")
                    && matches!(s.attribute(Some(W), "default"), Some("1" | "true"))
            })
            .and_then(|s| s.attribute(Some(W), "styleId").map(str::to_owned)),
    }
}

/// The style element of `id` as docx-agent's dict of styles holds it: the last one with
/// that id.
fn style_by_id<'d>(docx: &'d Docx, id: &str) -> Option<&'d Arc<Element>> {
    docx.styles()
        .iter()
        .rev()
        .find(|s| s.attribute(Some(W), "styleId") == Some(id))
}

fn based_on(style: &Element) -> Option<String> {
    style
        .find(Some(W), "basedOn")
        .and_then(|b| b.attribute(Some(W), "val"))
        .map(str::to_owned)
}

/// `(numId, ilvl)` a paragraph style numbers its paragraphs with, as Word numbers them,
/// or `None` (docx-agent's `style_list`): the style's own `w:numPr` or its `basedOn`
/// chain's; `numId` 0, an instance the document lacks, one naming no definition, and a
/// level tied to another style number nothing.
pub(crate) fn style_list(docx: &Docx, style_id: Option<&str>) -> Option<(i64, i64)> {
    let mut current = style_or_default(docx, style_id);
    let mut seen: Vec<String> = Vec::new();
    let (mut num_id, mut level, mut holder) = (None, None, None);
    while let Some(id) = current.take() {
        if seen.contains(&id) {
            break;
        }
        let Some(node) = style_by_id(docx, &id) else { break };
        if let Some(numbering) = find_path(node, &["pPr", "numPr"]) {
            if num_id.is_none() {
                if let Some(r) = numbering.find(Some(W), "numId") {
                    num_id = r.attribute(Some(W), "val").and_then(py_int);
                    holder = Some(id.clone());
                }
            }
            if level.is_none() {
                if let Some(r) = numbering.find(Some(W), "ilvl") {
                    level = r.attribute(Some(W), "val").and_then(py_int);
                }
            }
        }
        current = based_on(node);
        seen.push(id);
    }
    let level = level.unwrap_or(0);
    let lists = Lists::read(docx);
    let num_id = num_id.filter(|n| *n != 0 && lists.has(*n))?;
    let abstract_id = lists.abstract_of(num_id)?;
    if !has_abstract(docx, abstract_id) {
        return None;
    }
    let lvl = lists.level(docx, num_id, level);
    let tied = lvl
        .as_ref()
        .and_then(|l| l.find(Some(W), "pStyle"))
        .and_then(|s| s.attribute(Some(W), "val"))
        .filter(|v| !v.is_empty());
    if tied.is_some_and(|t| Some(t) != holder.as_deref()) {
        return None;
    }
    Some((num_id, level))
}

/// Whether the numbering part has definition `abstract_id`.
pub(crate) fn has_abstract(docx: &Docx, abstract_id: i64) -> bool {
    numbering_root(docx).is_some_and(|root| super::numbering::abstract_place(&root, abstract_id).is_some())
}

/// The numbering part's root, if there is one.
pub(crate) fn numbering_root(docx: &Docx) -> Option<Arc<Element>> {
    let part = ooxml_word_model::parts::related(&docx.package, &docx.main, super::numbering::REL_NUMBERING).ok()??;
    let tree = docx.package.tree(&part).ok()??;
    Some(Arc::clone(tree.root()))
}

/// A paragraph style's indent (`w:ind`), its own or its `basedOn` chain's.
pub(crate) fn style_indent(docx: &Docx, style_id: Option<&str>) -> Option<Arc<Element>> {
    let mut current = style_id.map(str::to_owned);
    let mut seen: Vec<String> = Vec::new();
    while let Some(id) = current.take() {
        if seen.contains(&id) {
            break;
        }
        let node = style_by_id(docx, &id)?;
        if let Some(found) = node.find(Some(W), "pPr").and_then(|p| p.find(Some(W), "ind")) {
            return Some(Arc::clone(found));
        }
        current = based_on(node);
        seen.push(id);
    }
    None
}

/// An indent's `(left, hanging - firstLine)` in twips.
fn form(node: &Element) -> (i64, i64) {
    let twips = |names: &[&str]| {
        names
            .iter()
            .find_map(|n| node.attribute(Some(W), n).and_then(py_int))
            .unwrap_or(0)
    };
    (twips(&["left", "start"]), twips(&["hanging"]) - twips(&["firstLine"]))
}

/// Twips in points as Python's `:g` writes `twips / 20` (at most two decimals, none
/// trailing).
fn points(twips: i64) -> String {
    let sign = if twips < 0 { "-" } else { "" };
    let magnitude = twips.unsigned_abs();
    let (whole, rest) = (magnitude / 20, (magnitude % 20) * 5);
    if rest == 0 {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{}", format!("{rest:02}").trim_end_matches('0'))
    }
}

/// The note for a paragraph given direct numbering over a style that numbers it with
/// another indent (docx-agent's `indent_note`); `style_ind` the style's indent.
pub(crate) fn indent_note(docx: &Docx, (num_id, level): (i64, i64), style_ind: Option<&Element>) -> Option<String> {
    let lvl = Lists::read(docx).level(docx, num_id, level)?;
    let level_ind = find_path(&lvl, &["pPr", "ind"])?;
    let style_ind = style_ind?;
    if form(level_ind) == form(style_ind) {
        return None;
    }
    let (left, hanging) = form(level_ind);
    Some(format!(
        "direct numbering: the paragraph takes its list level's indent (left {} pt, hanging {} pt), not its style's",
        points(left),
        points(hanging)
    ))
}

/// The paragraph style applied to `paragraph`: the one named if the document has it, else
/// the default (docx-agent's `_OutlineLevels.style_id`).
#[must_use]
pub fn applied_style(docx: &Docx, paragraph: &Element) -> Option<String> {
    let named = paragraph_style_id(paragraph).filter(|n| style_by_id(docx, n).is_some());
    named.or_else(|| {
        docx.default_style("paragraph")
            .and_then(|s| s.attribute(Some(W), "styleId"))
            .map(str::to_owned)
    })
}

/// The style that numbers `paragraph`, if its applied style numbers its paragraphs
/// (docx-agent's `word_lists` refusal of `remove`).
#[must_use]
pub fn numbered_by_style(docx: &Docx, paragraph: &Element) -> Option<String> {
    let style = applied_style(docx, paragraph)?;
    style_list(docx, Some(&style)).map(|_| style)
}

/// A style as docx-agent's `Styles` lists it: (id, name, based on, default).
fn paragraph_styles(docx: &Docx) -> Vec<(String, Option<String>, Option<String>, bool)> {
    docx.styles()
        .iter()
        .filter(|s| ooxml_word_model::docx::style_kind(s) == "paragraph")
        .map(|s| {
            let default = matches!(
                s.attribute(Some(W), "default").unwrap_or("0").to_lowercase().as_str(),
                "1" | "true" | "on"
            );
            (
                s.attribute(Some(W), "styleId").unwrap_or("").to_owned(),
                ooxml_word_model::docx::style_name(s).map(str::to_owned),
                based_on(s),
                default,
            )
        })
        .collect()
}

/// Names of paragraph styles that number nothing and fit where `style_id` stands, best
/// first, at most `limit` (docx-agent's `unnumbered_styles_like`).
#[must_use]
pub fn unnumbered_styles_like(docx: &Docx, style_id: &str, limit: usize) -> Vec<String> {
    use ooxml_word_model::docx::outline::style_outline_level;
    let styles = paragraph_styles(docx);
    let Some(numbered) = styles.iter().find(|s| s.0 == style_id) else {
        return Vec::new();
    };
    let level = |id: &str| style_outline_level(docx, Some(id));
    let numbered_level = level(&numbered.0);
    let mut chain: Vec<String> = Vec::new();
    let mut current = numbered.2.clone();
    while let Some(id) = current.take() {
        let Some(style) = styles.iter().find(|s| s.0 == id) else {
            break;
        };
        if chain.contains(&id) {
            break;
        }
        current = style.2.clone();
        chain.push(id);
    }
    let rank = |s: &(String, Option<String>, Option<String>, bool)| -> Option<u8> {
        if s.2.as_deref() == Some(style_id) {
            Some(0)
        } else if numbered_level.is_some() && level(&s.0) == numbered_level {
            Some(1)
        } else if chain.contains(&s.0) {
            Some(2)
        } else if numbered.2.is_some() && s.2 == numbered.2 {
            Some(3)
        } else if s.3 {
            Some(4)
        } else {
            None
        }
    };
    let mut ranked: Vec<(u8, usize)> = styles
        .iter()
        .enumerate()
        .filter(|(_, s)| s.0 != style_id)
        .filter_map(|(k, s)| rank(s).map(|r| (r, k)))
        .collect();
    ranked.sort_unstable();
    let mut out = Vec::new();
    for (_, k) in ranked {
        let Some(style) = styles.get(k) else { continue };
        if style_list(docx, Some(&style.0)).is_none() {
            out.push(
                style
                    .1
                    .clone()
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| style.0.clone()),
            );
        }
        if out.len() >= limit {
            break;
        }
    }
    out
}
