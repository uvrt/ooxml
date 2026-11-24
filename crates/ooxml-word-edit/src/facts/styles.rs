//! Owns: `check`'s `styles_not_allowed` and `styles_unknown` (docx-agent's `layout_facts`
//! `_Facts.style_list`, with `check`'s `styles`) read from the document alone: each
//! paragraph or table style in use (of every story but the comments) that a list of allowed
//! styles does not name, with the blocks in it, most used first; and the names in the list
//! that no style of the document has.
//!
//! Must not: read pages (the tool layer places the entries) or cap the blocks.

use super::style_names;
use crate::names::W;
use crate::{Result, WordDoc};
use ooxml_word_model::docx::find_path;
use std::collections::{HashMap, HashSet};

/// A paragraph or table style in use that the list does not allow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleUse {
    /// Its name (its id where it has none or no style has the id).
    pub style: String,
    /// `paragraph` or `table`.
    pub kind: &'static str,
    /// The blocks in it, by id, in document order.
    pub blocks: Vec<String>,
}

/// The styles in use a list does not allow, and the names in it no style has.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StyleList {
    /// The styles in use not allowed, most blocks first (in the order first used where
    /// they have as many).
    pub not_allowed: Vec<StyleUse>,
    /// The names allowed that no style of the document has, in the list's order.
    pub unknown: Vec<String>,
}

/// The styles in use that `allowed` (style names, aliases or ids, as `word_styles` finds
/// them) does not name: a paragraph with no style of its own is in the default paragraph
/// style, a table likewise.
///
/// # Errors
/// When the document cannot be read.
pub fn style_list(doc: &WordDoc, allowed: &[String]) -> Result<StyleList> {
    let docx = doc.docx()?;
    let mut ids: HashSet<String> = HashSet::new();
    let mut unknown = Vec::new();
    for name in allowed {
        match docx.find_any_style(name) {
            Some(style) => {
                ids.insert(style.attribute(Some(W), "styleId").unwrap_or_default().to_owned());
            }
            None => unknown.push(name.clone()),
        }
    }
    let default_id = |kind: &str| {
        docx.default_style(kind)
            .map(|s| s.attribute(Some(W), "styleId").unwrap_or_default().to_owned())
    };
    let (default_paragraph, default_table) = (default_id("paragraph"), default_id("table"));
    let mut groups: Vec<(&'static str, String, Vec<String>)> = Vec::new();
    let mut place: HashMap<(&'static str, String), usize> = HashMap::new();
    let mut count = |kind: &'static str, style: String, block: &str| {
        let key = (kind, style);
        let at = *place.entry(key.clone()).or_insert_with(|| {
            groups.push((kind, key.1.clone(), Vec::new()));
            groups.len() - 1
        });
        if let Some(group) = groups.get_mut(at) {
            group.2.push(block.to_owned());
        }
    };
    for story in &docx.parts {
        if story.story == "comments" {
            continue;
        }
        for paragraph in story.ids.paragraphs().iter() {
            let named = find_path(&paragraph.element, &["pPr", "pStyle"]);
            let style = match named {
                Some(node) => node.attribute(Some(W), "val").map(str::to_owned),
                None => default_paragraph.clone(),
            };
            if let Some(style) = style.filter(|s| !ids.contains(s)) {
                count("paragraph", style, &paragraph.id);
            }
        }
        let tables = story.ids.tables();
        for table in tables.placed().filter_map(|t| tables.head(t.ordinal)) {
            let named = find_path(&table.element, &["tblPr", "tblStyle"]);
            let style = match named {
                Some(node) => node.attribute(Some(W), "val").map(str::to_owned),
                None => default_table.clone(),
            };
            if let Some(style) = style.filter(|s| !ids.contains(s)) {
                count("table", style, &table.id);
            }
        }
    }
    let names = style_names(&docx);
    let mut not_allowed: Vec<StyleUse> = groups
        .into_iter()
        .map(|(kind, id, blocks)| StyleUse {
            style: names.get(&id).cloned().unwrap_or(id),
            kind,
            blocks,
        })
        .collect();
    not_allowed.sort_by_key(|u| std::cmp::Reverse(u.blocks.len()));
    Ok(StyleList { not_allowed, unknown })
}
