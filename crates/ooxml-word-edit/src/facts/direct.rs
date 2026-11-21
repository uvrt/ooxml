//! Owns: `check`'s `direct_formatting` (docx-agent's `layout_facts` `_Facts.direct_formatting`,
//! round 16 F8) read from the document alone: per paragraph (of every story but the
//! comments, the notes' separators left out), the properties set on the paragraph or on all
//! its text that make it differ from what its styles give it, in `word_format`'s names, and
//! those set on only some of its text -- most properties first, in document order.
//!
//! Must not: read pages (the tool layer places the entries), or count a table's keeps (a
//! row's deciding paragraph's keep with the next is the table's, `keep_together`'s).

use super::style_names;
use crate::names::W;
use crate::{Result, WordDoc};
use ooxml_base::pyfmt::round2;
use ooxml_word_model::docx::formatting::{Cell, EffectiveParagraph, EffectiveRun, Formatting};
use ooxml_word_model::docx::keeps::{row_leads, Keeps};
use ooxml_word_model::docx::text::{text_runs, View};
use ooxml_word_model::docx::{at_path, StoryPart};
use ooxml_xml::Element;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A property's value as `word_format` names it.
#[derive(Debug, Clone, PartialEq)]
pub enum FormatValue {
    /// None set (`null`).
    Unset,
    /// On or off.
    Flag(bool),
    /// Points, or a multiple of lines.
    Number(f64),
    /// A name (`center`, a face, `FF0000`).
    Name(String),
    /// A line spacing in points with its rule (`exact`, `atLeast`).
    Ruled(String, f64),
}

/// A paragraph whose direct formatting makes it differ from its styles.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectFormatting {
    /// Its id.
    pub para: String,
    /// Its paragraph style's name (the default paragraph style's where none is named).
    pub style: Option<String>,
    /// Each property set on it or on all its text, its direct value and its styles'.
    pub props: Vec<(&'static str, FormatValue, FormatValue)>,
    /// The run properties set on only some of its text.
    pub in_part: Vec<&'static str>,
}

/// Run properties and the `w:rPr` children that declare them (docx-agent's `_RUN_DECLARES`).
const RUN_DECLARES: [(&str, &[&str]); 8] = [
    ("bold", &["b", "bCs"]),
    ("italic", &["i", "iCs"]),
    ("underline", &["u"]),
    ("strike", &["strike"]),
    ("size", &["sz", "szCs"]),
    ("font", &["rFonts"]),
    ("color", &["color"]),
    ("highlight", &["highlight"]),
];

/// The `w:pPr` children that make a paragraph's properties worth comparing.
const PARAGRAPH_DECLARES: [&str; 6] = ["jc", "spacing", "ind", "keepNext", "keepLines", "pageBreakBefore"];

/// The paragraphs whose direct formatting makes them differ from their styles: those with
/// most properties first, then those with some of their text set apart only, each in
/// document order.
///
/// # Errors
/// When the document cannot be read.
pub fn direct_formatting(doc: &WordDoc) -> Result<Vec<DirectFormatting>> {
    let docx = doc.docx()?;
    let formatting = Formatting::read(&docx);
    let names = style_names(&docx);
    let default = docx
        .default_style("paragraph")
        .and_then(|s| s.attribute(Some(W), "styleId"))
        .map(str::to_owned);
    let mut found: Vec<(usize, DirectFormatting)> = Vec::new();
    for story in &docx.parts {
        if story.story == "comments" {
            continue;
        }
        let mut look = Keeps::new(&formatting, story);
        let kept = kept_leads(story, &mut look);
        for paragraph in story.ids.paragraphs().iter() {
            if separator(story, &paragraph.path) {
                continue;
            }
            let leads = kept.contains(&*paragraph.path);
            let entry = direct(&formatting, &mut look, &paragraph.element, &paragraph.path, leads);
            if let Some((props, in_part)) = entry {
                let style = style_of(&paragraph.element, default.as_deref(), &names);
                found.push((
                    found.len(),
                    DirectFormatting {
                        para: paragraph.id.clone(),
                        style,
                        props,
                        in_part,
                    },
                ));
            }
        }
    }
    found.sort_by_key(|(order, e)| (e.props.is_empty(), std::cmp::Reverse(e.props.len()), *order));
    Ok(found.into_iter().map(|(_, e)| e).collect())
}

/// Whether the paragraph at `path` is in a note that is a separator (`w:type` set:
/// `separator`, `continuationSeparator`, `continuationNotice`).
fn separator(story: &StoryPart, path: &[u32]) -> bool {
    let Some(parent) = path.split_last().and_then(|(_, up)| at_path(story.root(), up)) else {
        return false;
    };
    (parent.name().is(Some(W), "footnote") || parent.name().is(Some(W), "endnote"))
        && parent.attribute(Some(W), "type").is_some_and(|t| !t.is_empty())
}

/// The places of the paragraphs each row of a table kept together keeps with the next by.
fn kept_leads(story: &StoryPart, look: &mut Keeps<'_>) -> HashSet<Vec<u32>> {
    let mut out = HashSet::new();
    for table in story.ids.tables().iter() {
        let leads = row_leads(&table.element, &table.path);
        let Some(places) = leads.into_iter().collect::<Option<Vec<_>>>() else {
            continue;
        };
        let keeps = |look: &mut Keeps<'_>, place: &Vec<u32>| {
            at_path(story.root(), place).is_some_and(|p| look.effective(p, place, true).keep_with_next)
        };
        let but_last = places.get(..places.len().saturating_sub(1)).unwrap_or_default();
        if places.len() >= 2 && but_last.iter().all(|p| keeps(look, p)) {
            out.extend(places);
        }
    }
    out
}

/// A paragraph's style's name: of its `w:pStyle`, else the default paragraph style's.
fn style_of(paragraph: &Element, default: Option<&str>, names: &HashMap<String, String>) -> Option<String> {
    let named = paragraph.find(Some(W), "pPr").and_then(|p| p.find(Some(W), "pStyle"));
    let id = match named {
        Some(node) => node.attribute(Some(W), "val"),
        None => default,
    }?;
    Some(names.get(id).cloned().unwrap_or_else(|| id.to_owned()))
}

type Found = (Vec<(&'static str, FormatValue, FormatValue)>, Vec<&'static str>);

/// A paragraph's direct properties and those on some of its text, `None` where it has
/// neither (docx-agent's `_direct`). `leads`: a row of a table kept together keeps with
/// the next by it.
fn direct(
    formatting: &Formatting,
    look: &mut Keeps<'_>,
    paragraph: &Arc<Element>,
    place: &[u32],
    leads: bool,
) -> Option<Found> {
    let mut props = Vec::new();
    let declares = paragraph
        .find(Some(W), "pPr")
        .is_some_and(|p| PARAGRAPH_DECLARES.iter().any(|t| p.find(Some(W), t).is_some()));
    if declares {
        let mine = look.effective(paragraph, place, true);
        let styled = look.effective(paragraph, place, false);
        for (name, value, base) in paragraph_values(&mine, &styled) {
            if value != base && !(name == "keep_with_next" && leads) {
                props.push((name, value, base));
            }
        }
    }
    let cell = look.cell(place);
    let in_part = run_props(formatting, paragraph, cell.as_ref(), &mut props);
    (!props.is_empty() || !in_part.is_empty()).then_some((props, in_part))
}

/// What a property overridden in some runs was set to: the characters it covers, each
/// value's, and the styles' value.
struct Overridden {
    name: &'static str,
    chars: usize,
    values: Vec<(FormatValue, usize)>,
    base: FormatValue,
}

/// Adds to `props` each run property set on all the paragraph's text (its most used value);
/// returns those set on only some of it.
fn run_props(
    formatting: &Formatting,
    paragraph: &Arc<Element>,
    cell: Option<&Cell>,
    props: &mut Vec<(&'static str, FormatValue, FormatValue)>,
) -> Vec<&'static str> {
    let runs = text_runs(paragraph, View::Current);
    let total: usize = runs.iter().map(|r| r.text).sum();
    let mut overridden: Vec<Overridden> = Vec::new();
    for run in &runs {
        let Some(properties) = at_path(paragraph, &run.path).and_then(|r| r.find(Some(W), "rPr")) else {
            continue;
        };
        let declared = |tags: &[&str]| tags.iter().any(|t| properties.find(Some(W), t).is_some());
        if !RUN_DECLARES.iter().any(|(_, tags)| declared(tags)) {
            continue;
        }
        let first = run.first_text.unwrap_or('a');
        let style = properties
            .find(Some(W), "rStyle")
            .and_then(|s| s.attribute(Some(W), "val"));
        let mine = formatting.run_properties(paragraph, Some(properties), first, cell);
        let styled = formatting.run_styled(paragraph, style, first, cell);
        for (name, tags) in RUN_DECLARES {
            if !declared(tags) {
                continue;
            }
            let (value, base) = (run_value(&mine, name), run_value(&styled, name));
            if value == base {
                continue;
            }
            let at = match overridden.iter().position(|o| o.name == name) {
                Some(at) => at,
                None => {
                    overridden.push(Overridden {
                        name,
                        chars: 0,
                        values: Vec::new(),
                        base,
                    });
                    overridden.len() - 1
                }
            };
            let Some(seen) = overridden.get_mut(at) else { continue };
            seen.chars += run.text;
            match seen.values.iter_mut().find(|(v, _)| *v == value) {
                Some((_, count)) => *count += run.text,
                None => seen.values.push((value, run.text)),
            }
        }
    }
    let mut in_part = Vec::new();
    for seen in overridden {
        if seen.chars >= total {
            // Python's max: the first of the most used.
            let mut best: Option<&(FormatValue, usize)> = None;
            for entry in &seen.values {
                if best.is_none_or(|b| entry.1 > b.1) {
                    best = Some(entry);
                }
            }
            if let Some((value, _)) = best {
                props.push((seen.name, value.clone(), seen.base));
            }
        } else {
            in_part.push(seen.name);
        }
    }
    in_part
}

fn optional(text: Option<&String>) -> FormatValue {
    text.map_or(FormatValue::Unset, |t| FormatValue::Name(t.clone()))
}

/// A run property's value as docx-agent's `EffectiveRun` holds it.
fn run_value(run: &EffectiveRun, name: &str) -> FormatValue {
    match name {
        "bold" => FormatValue::Flag(run.bold),
        "italic" => FormatValue::Flag(run.italic),
        "underline" => optional(run.underline.as_ref()),
        "strike" => FormatValue::Flag(run.strike),
        "size" => FormatValue::Number(run.size),
        "font" => optional(run.font.as_ref()),
        "color" => FormatValue::Name(run.color.clone()),
        _ => optional(run.highlight.as_ref()),
    }
}

/// `(word_format's name, direct, styles')` for each paragraph property compared
/// (docx-agent's `_paragraph_values`).
fn paragraph_values(
    mine: &EffectiveParagraph,
    styled: &EffectiveParagraph,
) -> [(&'static str, FormatValue, FormatValue); 10] {
    let line = |e: &EffectiveParagraph| match e.line_spacing {
        None => FormatValue::Unset,
        Some(s) if e.line_rule == "auto" => FormatValue::Number(round2(s)),
        Some(s) => FormatValue::Ruled(e.line_rule.clone(), round2(s)),
    };
    let first = |e: &EffectiveParagraph| FormatValue::Number(round2(e.first_line - e.hanging));
    let number = FormatValue::Number;
    let flag = FormatValue::Flag;
    [
        (
            "alignment",
            optional(mine.alignment.as_ref()),
            optional(styled.alignment.as_ref()),
        ),
        ("space_before", number(mine.space_before), number(styled.space_before)),
        ("space_after", number(mine.space_after), number(styled.space_after)),
        ("line_spacing", line(mine), line(styled)),
        ("indent_left", number(mine.indent_left), number(styled.indent_left)),
        ("indent_right", number(mine.indent_right), number(styled.indent_right)),
        ("indent_first", first(mine), first(styled)),
        ("keep_with_next", flag(mine.keep_with_next), flag(styled.keep_with_next)),
        ("keep_lines", flag(mine.keep_together), flag(styled.keep_together)),
        (
            "page_break_before",
            flag(mine.page_break_before),
            flag(styled.page_break_before),
        ),
    ]
}
