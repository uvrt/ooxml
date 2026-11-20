//! Owns: Keep Source Formatting and Merge Formatting on copied blocks, as docx-agent's
//! importer writes them (`_formatting` for those policies, `_flatten`, `_run_values`,
//! `_keeps_structure`): a table's style brought by name; every paragraph made the
//! destination's default paragraph style, and the source's effective formatting where it
//! differs from the destination's base (all of it, or under `merge` only bold, italic and
//! underline) written as direct formatting on the paragraph, its mark and its runs, theme
//! fonts and colours concrete where the themes differ; under `merge` a heading or a list
//! item keeping a destination style of its kind instead (a built-in one added as Word
//! writes it).
//!
//! Not ported yet ([`EditError::Unsupported`]): a paragraph a style's list would be
//! written on.
//!
//! Must not: write but into the copies and through [`Styles`].

use super::canonical::same_copy;
use super::cascade::{is_toggle, on, Props, Tag};
use super::styles::Styles;
use super::StylePolicy;
use crate::marks::insert_in_order;
use crate::names::W;
use crate::review::space::Space;
use crate::review::xml::make;
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::casefold;
use ooxml_xml::{Element, QName};
use std::sync::Arc;

/// Whether a style name is `heading 1` to `heading 9` (any case).
pub(super) fn is_heading(name: &str) -> bool {
    let name = name.to_lowercase();
    name.strip_prefix("heading ")
        .is_some_and(|n| n.len() == 1 && n.bytes().all(|b| (b'1'..=b'9').contains(&b)))
}

/// Whether a style name is one of Word's list styles (`_LIST_STYLE`, any case).
pub(super) fn is_list(name: &str) -> bool {
    let name = name.to_lowercase();
    if name == "list paragraph" {
        return true;
    }
    ["list bullet", "list number", "list continue"].iter().any(|stem| {
        name.strip_prefix(stem).is_some_and(|rest| {
            rest.is_empty()
                || (rest.len() == 2
                    && rest.starts_with(' ')
                    && ('2'..='5').contains(&rest.chars().nth(1).unwrap_or(' ')))
        })
    })
}

/// The text of a paragraph's `w:t` elements.
pub(super) fn t_text(paragraph: &Element) -> String {
    let mut out = String::new();
    let mut stack = vec![paragraph];
    while let Some(node) = stack.pop() {
        if node.name().is(Some(W), "t") {
            out.push_str(&node.text_content());
            continue;
        }
        stack.extend(
            node.child_elements()
                .map(|c| &**c)
                .collect::<Vec<_>>()
                .into_iter()
                .rev(),
        );
    }
    out
}

/// The `w:val` of a child of `properties`.
fn child_val(work: &Work, properties: Option<Id>, local: &str) -> Option<String> {
    properties
        .and_then(|p| work.find(p, W, local))
        .and_then(|n| work.get(n, Some(W), "val"))
        .map(str::to_owned)
}

/// The destination's base formatting and the copy's policy, for one block.
struct Base {
    paragraph: Props,
    run: Props,
    merge: bool,
}

impl Styles<'_> {
    /// One copied block under `keep_source` or `merge`.
    ///
    /// # Errors
    /// What [`Styles::style`] refuses or leaves to the host.
    pub(super) fn flatten_block(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        space: &mut Space,
        work: &mut Work,
        element: Id,
    ) -> Result<()> {
        let (paragraph, run) = self.bases();
        let base = Base {
            paragraph,
            run,
            merge: self.policies.styles == StylePolicy::Merge,
        };
        for node in work.iter(element, W, &["tblStyle"]) {
            self.point(sink, space, work, node, "table")?;
        }
        for paragraph in work.iter(element, W, &["p"]) {
            if base.merge && self.keeps_structure(sink, space, work, paragraph)? {
                continue;
            }
            self.flatten(work, paragraph, &base)?;
        }
        Ok(())
    }

    /// The list a paragraph's style chain gives it, if any.
    fn style_list(&self, style: Option<&str>) -> bool {
        self.cascade.chain(style, "paragraph").iter().any(|level| {
            level
                .find(Some(W), "pPr")
                .and_then(|p| p.find(Some(W), "numPr"))
                .is_some()
        })
    }

    /// Merge Formatting's heading or list item: it keeps a destination style of its kind
    /// (`_keeps_structure`).
    fn keeps_structure(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        space: &mut Space,
        work: &mut Work,
        paragraph: Id,
    ) -> Result<bool> {
        let properties = work.find(paragraph, W, "pPr");
        let Some(node) = properties.and_then(|p| work.find(p, W, "pStyle")) else {
            return Ok(false);
        };
        let Some(id) = work
            .get(node, Some(W), "val")
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
        else {
            return Ok(false);
        };
        let name = self
            .cascade
            .name(&id)
            .filter(|n| !n.is_empty())
            .unwrap_or(&id)
            .to_owned();
        let chain = self.cascade.chain(Some(&id), "paragraph");
        let outline: Option<String> = chain
            .iter()
            .find_map(|level| level.find(Some(W), "pPr").and_then(|p| p.find(Some(W), "outlineLvl")))
            .and_then(|o| o.attribute(Some(W), "val").map(str::to_owned));
        let named_heading = is_heading(&name);
        let heading = named_heading || outline.as_deref().is_some_and(|o| o != "9");
        let listed = is_list(&name) || self.style_list(Some(&id));
        if !(heading || listed) {
            return Ok(false);
        }
        let mut wanted = self.policies.names.get(&casefold(&name)).cloned();
        if wanted.is_none() && !named_heading && heading {
            if let Some(level) = outline
                .as_deref()
                .filter(|o| !o.is_empty() && o.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|o| o.parse::<u64>().ok())
                .filter(|l| *l <= 8)
            {
                wanted = Some(format!("heading {}", level + 1));
            }
        }
        let target = match self.destination_style(sink, space, wanted.as_deref().unwrap_or(&name), Some("paragraph")) {
            Ok(Some(target)) => target,
            Ok(None) | Err(EditError::Refused(_)) => return Ok(false),
            Err(other) => return Err(other),
        };
        let numbered = properties.and_then(|p| work.find(p, W, "numPr")).is_some();
        if !numbered && listed && self.style_list(Some(&id)) {
            return Err(EditError::Unsupported("a copied style's list".to_owned()));
        }
        work.set(node, Arc::new(QName::new(Some("w"), "val", Some(W))), &target);
        for run_style in work.iter(paragraph, W, &["rStyle"]) {
            self.point(sink, space, work, run_style, "character")?;
        }
        Ok(true)
    }

    /// One paragraph made the default style with the source's look direct (`_flatten`).
    fn flatten(&self, work: &mut Work, paragraph: Id, base: &Base) -> Result<()> {
        let properties = work.find(paragraph, W, "pPr");
        let style = child_val(work, properties, "pStyle");
        let numbered = properties.and_then(|p| work.find(p, W, "numPr")).is_some();
        if numbered || self.style_list(style.as_deref()) {
            return Err(EditError::Unsupported("a copied list".to_owned()));
        }
        let orders = self.orders();
        let new_properties = make(work, "pPr", &[]);
        if !base.merge {
            let direct = properties.and_then(|p| work.build_element(p));
            for (tag, node) in self.cascade.paragraph(style.as_deref(), direct.as_deref()).iter() {
                if base.paragraph.get(tag).is_some_and(|b| same_copy(b, node)) {
                    continue;
                }
                let adopted = work.adopt_copy(node).ok_or_else(|| EditError::Path("pPr".to_owned()))?;
                insert_in_order(work, new_properties, adopted, orders);
            }
        }
        let mark = properties
            .and_then(|p| work.find(p, W, "rPr"))
            .and_then(|m| work.build_element(m));
        let mark_values = self.run_values(work, (style.as_deref(), None), mark.as_deref(), base)?;
        if !work.children(mark_values).is_empty() {
            insert_in_order(work, new_properties, mark_values, orders);
        }
        if let Some(p) = properties {
            work.remove(p);
        }
        if !work.children(new_properties).is_empty() {
            work.insert(paragraph, 0, new_properties);
        }
        for run in work.iter(paragraph, W, &["r"]) {
            let direct = work.find(run, W, "rPr");
            let run_style = child_val(work, direct, "rStyle");
            let built = direct.and_then(|d| work.build_element(d));
            let values = self.run_values(work, (style.as_deref(), run_style.as_deref()), built.as_deref(), base)?;
            if let Some(d) = direct {
                work.remove(d);
            }
            if !work.children(values).is_empty() {
                work.insert(run, 0, values);
            }
        }
        Ok(())
    }

    /// A run's (or the mark's) properties as written (`_run_values`), detached.
    fn run_values(
        &self,
        work: &mut Work,
        (paragraph_style, run_style): (Option<&str>, Option<&str>),
        direct: Option<&Element>,
        base: &Base,
    ) -> Result<Id> {
        let out = make(work, "rPr", &[]);
        for (tag, node) in self.cascade.run(paragraph_style, run_style, direct).iter() {
            if base.merge && !kept_by_merge(tag, node) {
                continue;
            }
            let ours = base.run.get(tag);
            if ours.is_some_and(|b| same_copy(b, node)) {
                continue;
            }
            if is_toggle(tag) && !on(node) && ours.is_none() {
                continue;
            }
            let value = self.themes.concrete(node);
            let adopted = work
                .adopt_copy(&value)
                .ok_or_else(|| EditError::Path("rPr".to_owned()))?;
            insert_in_order(work, out, adopted, self.orders());
        }
        Ok(out)
    }
}

/// What Merge Formatting keeps of a run's formatting: bold and italic when on, an
/// underline but none.
fn kept_by_merge(tag: &Tag, node: &Element) -> bool {
    if tag.0.as_deref() != Some(W) {
        return false;
    }
    match tag.1.as_str() {
        "u" => {
            node.attribute(Some(W), "val")
                .filter(|v| !v.is_empty())
                .unwrap_or("single")
                != "none"
        }
        "b" | "i" => on(node),
        _ => false,
    }
}
