//! Owns: a Markdown list as insert_markdown writes it (docx-agent's `Writer.list` and
//! `_abstract`, over `edit/numbering.py`'s `style_list`, `add_num`, `add_restart`,
//! `copy_abstract`, `builtin_abstract` and `_document_definition`): items in the level's
//! list style; where that style numbers itself at the list's level its numbering applies
//! (the insertion's first list at a level counting on, a later one or one starting past 1
//! restarting with an instance of its own on its first item); otherwise one instance per
//! list over a definition made for the insertion -- a copy of the document's own nine-level
//! one of the kind, else Word's own.
//!
//! Must not: write but through the writer and the sink.

use super::writer::{properties, Writer};
use crate::lists::numbering::{add_builtin, add_num, add_restart, copy_abstract, part_of};
use crate::names::W;
use crate::review::xml::make;
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::Result;
use ooxml_word_model::docx::numbering::Lists;
use ooxml_word_model::docx::{child_val, find_path, Docx};
use ooxml_word_model::markdown::model::{Block, ListBlock};

/// The deepest list level.
const MAX_LEVEL: i64 = 8;
impl Writer<'_> {
    /// The paragraphs of `list` at `depth` and the blocks inside its items.
    pub(crate) fn list(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        work: &mut Work,
        list: &ListBlock,
        depth: i64,
    ) -> Result<Vec<Id>> {
        let level = depth.min(MAX_LEVEL);
        let construct = if list.ordered { "ordered_list" } else { "bullet_list" };
        let style = self.style(sink, construct, depth + 1)?;
        let docx = sink.docx()?;
        let lists = Lists::read(&docx);
        let by_style = style
            .as_deref()
            .and_then(|s| style_list(&docx, &lists, Some(s)))
            .filter(|(_, l)| *l == level);
        let main = docx.main.clone();
        let mut num_id: Option<i64> = None;
        let mut key = None;
        match by_style {
            None => {
                let abstract_id = self.abstract_for(sink, list.ordered)?;
                let start: &[(i64, i64)] = if list.ordered { &[(level, list.start)] } else { &[] };
                num_id = Some(add_num(sink, &main, abstract_id, start)?);
            }
            Some((style_num, style_level)) => {
                let abstract_id = lists.abstract_of(style_num);
                let canonical = lists.canonical(abstract_id);
                key = Some(canonical);
                let counting = self.counting.entry(canonical).or_default();
                let restarts = (list.ordered && list.start != 1) || counting.contains(&style_level);
                counting.insert(style_level);
                if restarts {
                    let restarted = (abstract_id.unwrap_or_default(), style_num, style_level);
                    num_id = Some(add_restart(sink, &main, restarted, list.start, style.as_deref())?);
                    self.numbered = true;
                }
            }
        }
        let mut out = Vec::new();
        for item in &list.items {
            let (inline, rest): (&[_], &[Block]) = match item.split_first() {
                Some((Block::Paragraph(inline), rest)) => (inline, rest),
                _ => (&[], item),
            };
            let paragraph = self.paragraph(sink, work, inline, style.as_deref(), None)?;
            if let Some(num) = num_id {
                set_numbering(work, paragraph, num, by_style.map_or(level, |(_, l)| l));
                if by_style.is_some() {
                    num_id = None;
                }
            }
            if let (Some((_, style_level)), Some(key)) = (by_style, key) {
                // an item restarts the deeper levels' counts
                if let Some(counting) = self.counting.get_mut(&key) {
                    counting.retain(|n| *n <= style_level);
                }
            }
            out.push(paragraph);
            for inner in rest {
                if let Block::List(nested) = inner {
                    out.extend(self.list(sink, work, nested, depth + 1)?);
                } else {
                    self.in_list += 1;
                    let made = self.blocks(sink, work, std::slice::from_ref(inner));
                    self.in_list -= 1;
                    out.extend(made?);
                }
            }
        }
        Ok(out)
    }

    /// The abstract definition this insertion's lists of a kind use: a copy of the
    /// document's own nine-level one, else Word's own.
    fn abstract_for(&mut self, sink: &mut EditSink<'_, '_>, ordered: bool) -> Result<i64> {
        if let Some(found) = self.abstracts.get(&ordered) {
            return Ok(*found);
        }
        let docx = sink.docx()?;
        let main = docx.main.clone();
        let lists = Lists::read(&docx);
        let source = document_definition(&docx, &lists, ordered);
        let abstract_id = match source {
            Some(source) => {
                let part = part_of(sink, &main)?.ok_or_else(|| crate::EditError::NoPart("numbering".to_owned()))?;
                copy_abstract(sink, &main, &part, source)?
            }
            None => {
                let kind = if ordered { "number" } else { "bullet" };
                add_builtin(sink, &main, kind, |id| format!("markdown\0{kind}\0{id}"))?
            }
        };
        self.abstracts.insert(ordered, abstract_id);
        Ok(abstract_id)
    }
}

/// Python's `int` of an attribute.
fn int_of(value: Option<&str>) -> Option<i64> {
    ooxml_word_model::xmlread::py_int(value?)
}

/// `set_numbering`: the paragraph's own `w:numPr` (after its style).
fn set_numbering(work: &mut Work, paragraph: Id, num_id: i64, level: i64) {
    let properties = properties(work, paragraph);
    let numbering = make(work, "numPr", &[]);
    let at = usize::from(work.find(properties, W, "pStyle").is_some());
    work.insert(properties, at, numbering);
    let ilvl = make(work, "ilvl", &[("val", &level.to_string())]);
    work.append(numbering, ilvl);
    let num = make(work, "numId", &[("val", &num_id.to_string())]);
    work.append(numbering, num);
}

/// `style_list`: `(numId, ilvl)` a paragraph style numbers its paragraphs with, as Word
/// numbers them, or none.
fn style_list(docx: &Docx, lists: &Lists, style: Option<&str>) -> Option<(i64, i64)> {
    let mut current = style.map(str::to_owned);
    let mut seen = std::collections::HashSet::new();
    let (mut num_id, mut level, mut holder) = (None, None, None);
    while let Some(id) = current.take() {
        if !seen.insert(id.clone()) {
            break;
        }
        let Some(node) = docx.style(&id) else { break };
        if let Some(numbering) = find_path(node, &["pPr", "numPr"]) {
            if num_id.is_none() {
                if let Some(n) = numbering.child_elements().find(|c| c.name().is(Some(W), "numId")) {
                    num_id = int_of(n.attribute(Some(W), "val"));
                    holder = Some(id.clone());
                }
            }
            if level.is_none() {
                if let Some(l) = numbering.child_elements().find(|c| c.name().is(Some(W), "ilvl")) {
                    level = int_of(l.attribute(Some(W), "val"));
                }
            }
        }
        current = child_val(node, "basedOn").map(str::to_owned);
    }
    let level = level.unwrap_or(0);
    let num_id = num_id.filter(|n| *n != 0 && lists.has(*n))?;
    lists.abstract_of(num_id).and_then(|a| lists.abstract_element(a))?;
    let lvl = lists.level(docx, num_id, level);
    let tied = lvl
        .as_ref()
        .and_then(|l| child_val(l, "pStyle"))
        .filter(|v| !v.is_empty());
    if tied.is_some_and(|t| Some(t) != holder.as_deref()) {
        return None;
    }
    Some((num_id, level))
}

/// `_document_definition`: the abstract definition the document's own lists of a kind use
/// most -- nine levels, all bullets (or none), none tied to a style, not a list style's.
fn document_definition(docx: &Docx, lists: &Lists, ordered: bool) -> Option<i64> {
    let mut counts: Vec<(i64, usize)> = Vec::new();
    let mut parts = vec![docx.main.clone()];
    parts.extend(ooxml_word_model::parts::story_parts(&docx.package, &docx.main).unwrap_or_default());
    for part in parts {
        let Some(story) = docx.part(&part) else { continue };
        let mut stack = vec![story.root()];
        while let Some(element) = stack.pop() {
            if element.name().is(Some(W), "p") {
                let found =
                    find_path(element, &["pPr", "numPr", "numId"]).and_then(|n| int_of(n.attribute(Some(W), "val")));
                if let Some(a) = found.and_then(|n| lists.abstract_of(n)) {
                    match counts.iter_mut().find(|(k, _)| *k == a) {
                        Some(slot) => slot.1 += 1,
                        None => counts.push((a, 1)),
                    }
                }
            }
            let children: Vec<_> = element.child_elements().collect();
            stack.extend(children.into_iter().rev());
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    counts.into_iter().map(|(a, _)| a).find(|&a| {
        let Some(node) = lists.abstract_element(a) else {
            return false;
        };
        let has = |local: &str| node.child_elements().any(|c| c.name().is(Some(W), local));
        if has("styleLink") || has("numStyleLink") {
            return false;
        }
        let levels: Vec<_> = node.child_elements().filter(|c| c.name().is(Some(W), "lvl")).collect();
        if levels.len() < 9
            || levels
                .iter()
                .any(|l| l.child_elements().any(|c| c.name().is(Some(W), "pStyle")))
        {
            return false;
        }
        let formats: Vec<String> = levels
            .iter()
            .take(9)
            .map(|l| child_val(l, "numFmt").unwrap_or("decimal").to_owned())
            .collect();
        if ordered {
            formats.iter().all(|f| f != "bullet" && f != "none")
        } else {
            formats.iter().all(|f| f == "bullet")
        }
    })
}
