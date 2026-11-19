//! Owns: which body paragraphs a table of contents lists, at which levels (docx-agent's
//! `_toc_entries`, measured: `\o`'s range of style levels, `\t`'s named styles -- their
//! names split on the list separator of the document's language and found in it and in
//! English --, `\u`'s paragraph levels), a heading's text as an entry shows it
//! (`_heading_text`), and the language field style names are read in (`_field_language`).
//!
//! Must not: write anything.

use super::super::switches::toc_options;
use crate::names::W;
use crate::text::paragraph_text;
use crate::work::Work;
use ooxml_word_model::docx::outline::OutlineLevels;
use ooxml_word_model::docx::text::{LINE_BREAK, OBJECT};
use ooxml_word_model::docx::{find_path, style_name, Docx};
use ooxml_word_model::ids::ParagraphId;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// A paragraph a table of contents lists, at its level.
pub(crate) struct Heading {
    /// The paragraph.
    pub(crate) paragraph: ParagraphId,
    /// Its level, 1-9.
    pub(crate) level: i64,
}

/// A heading's text as its entry shows it: the paragraph's text, objects left out, line
/// and page breaks as spaces, trimmed.
pub(crate) fn heading_text(paragraph: &Arc<Element>) -> String {
    let work = Work::of(paragraph, Vec::new());
    let text = paragraph_text(&work, work.root()).replace(OBJECT, "");
    text.replace([LINE_BREAK, '\u{c}'], " ").trim().to_owned()
}

/// The language field style names are read in: the settings' `w:themeFontLang`, else
/// the default run language; `None` (English only) when neither says.
pub(crate) fn field_language(docx: &Docx) -> Option<String> {
    let val = |root: Option<Arc<Element>>, path: &[&str]| {
        root.as_deref()
            .and_then(|r| find_path(r, path))
            .and_then(|n| n.attribute(Some(W), "val"))
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    val(part_root(docx, SETTINGS), &["themeFontLang"])
        .or_else(|| val(part_root(docx, STYLES), &["docDefaults", "rPrDefault", "rPr", "lang"]))
}

/// The settings part's relationship type.
const SETTINGS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings";
/// The styles part's relationship type.
const STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";

/// The root of the main part's related part of `rel_type`, if there is one.
fn part_root(docx: &Docx, rel_type: &str) -> Option<Arc<Element>> {
    let name = ooxml_word_model::parts::related(&docx.package, &docx.main, rel_type).ok()??;
    let tree = docx.package.tree(&name).ok()??;
    Some(Arc::clone(tree.root()))
}

/// The paragraph style ids `\t`'s names find, with their levels (a style named twice:
/// the first).
fn named_styles(docx: &Docx, styles: &[(String, i64)]) -> HashMap<String, i64> {
    let mut named = HashMap::new();
    if styles.is_empty() {
        return named;
    }
    let mut stored: Vec<(String, String)> = Vec::new();
    for style in docx.styles() {
        let kind = style.attribute(Some(W), "type").unwrap_or("paragraph");
        if kind != "paragraph" {
            continue;
        }
        let id = style.attribute(Some(W), "styleId").unwrap_or("").to_owned();
        let aliases = find_path(style, &["aliases"])
            .and_then(|a| a.attribute(Some(W), "val"))
            .unwrap_or("");
        let names = style_name(style)
            .into_iter()
            .map(str::to_owned)
            .chain(aliases.split(',').map(|a| a.trim().to_owned()));
        for name in names.filter(|n| !n.is_empty()) {
            stored.push((name, id.clone()));
        }
    }
    let pairs: Vec<(&str, String)> = stored.iter().map(|(n, i)| (n.as_str(), i.clone())).collect();
    let language = field_language(docx);
    for (name, level) in styles {
        if let Some(found) = ooxml_style_names::find_style(name, &pairs, language.as_deref(), true) {
            named.entry(found).or_insert(*level);
        }
    }
    named
}

/// The body's paragraphs a TOC field with `instruction` lists, in order, with their
/// levels.
pub(crate) fn entries(docx: &Docx, instruction: &str) -> Vec<Heading> {
    let language = field_language(docx);
    let separator = ooxml_style_names::list_separator(language.as_deref(), ",");
    let options = toc_options(instruction, separator);
    let named = named_styles(docx, &options.styles);
    let levels = OutlineLevels::new(docx);
    let Some(body) = docx.body() else { return Vec::new() };
    let mut out = Vec::new();
    for paragraph in body.ids.paragraphs().iter() {
        let element = &paragraph.element;
        let style_id = levels.style_id(element);
        let in_range = |level: i64| options.outline.is_some_and(|(a, b)| a <= level && level <= b);
        let level = if options.own {
            let Some(found) = levels.effective(element) else {
                continue;
            };
            if in_range(found) {
                Some(found)
            } else {
                style_id.as_ref().and_then(|s| named.get(s).copied())
            }
        } else {
            style_id.as_ref().and_then(|s| named.get(s).copied()).or_else(|| {
                options
                    .outline
                    .and(levels.style(style_id.as_deref()))
                    .filter(|l| in_range(*l))
            })
        };
        let Some(level) = level else { continue };
        if heading_text(element).is_empty() {
            continue;
        }
        out.push(Heading { paragraph, level });
    }
    out
}
