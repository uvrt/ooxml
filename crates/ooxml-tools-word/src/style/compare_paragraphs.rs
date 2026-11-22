//! Owns: paragraphs compared as docx-agent's `edit/compare.py` compares them -- each
//! paragraph's style (by name, so another document's styles compare) and its direct
//! formatting in `word_format`'s names, what its `w:pPr` states and what most of its text's
//! runs state, with the `word_format` call that gives the reference's value -- and how a
//! cell's paragraphs pair with its reference cell's ([`align`], the module's
//! "Corresponding parts": by text, then by style, in order, repeats of a pattern after it).
//!
//! Must not: change a document, or read tables (`super::compare_tables` does).

use super::compare::{difference, fix, Block, Difference, Groups, Para, Side};
use crate::table_read::python_g;
use ooxml_base::pyfmt::round2;
use ooxml_word_edit::pairing::{self, Signature};
use ooxml_word_model::docx::at_path;
use ooxml_word_model::docx::find_path;
use ooxml_word_model::docx::formatting::{EffectiveParagraph, EffectiveRun};
use ooxml_word_model::docx::text::{paragraph_text, text_runs, View};
use ooxml_word_model::ids::W;
use ooxml_xml::Element;
use serde_json::{json, Map, Value};

/// The characters of a paragraph's text a missing or extra paragraph names.
const TEXT_START: usize = 40;

/// A paragraph's direct properties: `word_format`'s name, the `w:pPr` child and the
/// attributes that state it (none: the element itself).
pub(crate) const PARAGRAPH: [(&str, &str, &[&str]); 10] = [
    ("alignment", "jc", &[]),
    (
        "space_before",
        "spacing",
        &["before", "beforeLines", "beforeAutospacing"],
    ),
    ("space_after", "spacing", &["after", "afterLines", "afterAutospacing"]),
    ("line_spacing", "spacing", &["line"]),
    ("indent_left", "ind", &["left", "start"]),
    ("indent_right", "ind", &["right", "end"]),
    (
        "indent_first",
        "ind",
        &["firstLine", "hanging", "firstLineChars", "hangingChars"],
    ),
    ("keep_with_next", "keepNext", &[]),
    ("keep_lines", "keepLines", &[]),
    ("page_break_before", "pageBreakBefore", &[]),
];

/// A run's direct properties: `word_format`'s name and the `w:rPr` children that state it.
const RUN: [(&str, &[&str]); 8] = [
    ("bold", &["b", "bCs"]),
    ("italic", &["i", "iCs"]),
    ("underline", &["u"]),
    ("strike", &["strike"]),
    ("size", &["sz", "szCs"]),
    ("font", &["rFonts"]),
    ("color", &["color"]),
    ("highlight", &["highlight"]),
];

/// A cell's paragraphs paired with its reference cell's: the pairs to compare, the
/// reference's paragraphs left unpaired and the copy's.
pub(crate) struct Aligned {
    /// `(paragraph, reference)` pairs.
    pub(crate) pairs: Vec<(Para, Para)>,
    /// Indexes of the reference's paragraphs left unpaired.
    pub(crate) missing: Vec<usize>,
    /// Indexes of the copy's paragraphs left unpaired.
    pub(crate) extra: Vec<usize>,
}

fn signature(side: &Side<'_>, paragraph: &Para) -> Signature {
    pairing::signature(
        paragraph_style(side, &paragraph.0),
        &paragraph_text(&paragraph.0, View::Current),
    )
}

/// A missing or extra paragraph as reported: its style and the start of its text.
pub(crate) fn described(side: &Side<'_>, paragraph: &Para) -> Map<String, Value> {
    let text = pairing::collapse(&paragraph_text(&paragraph.0, View::Current));
    let chars: Vec<char> = text.chars().collect();
    let text = if chars.len() > TEXT_START {
        let start: String = chars.iter().take(TEXT_START).collect();
        format!("{}\u{2026}", start.trim_end_matches(crate::describe::py_space))
    } else {
        text
    };
    let mut out = Map::new();
    out.insert("paragraph_style".into(), json!(paragraph_style(side, &paragraph.0)));
    out.insert("text".into(), Value::from(text));
    out
}

/// A cell's paragraphs paired with its reference cell's (`ooxml_word_edit::pairing`).
pub(crate) fn align(mine_side: &Side<'_>, paragraphs: &[Para], theirs_side: &Side<'_>, reference: &[Para]) -> Aligned {
    let mine: Vec<Signature> = paragraphs.iter().map(|p| signature(mine_side, p)).collect();
    let theirs: Vec<Signature> = reference.iter().map(|p| signature(theirs_side, p)).collect();
    let found = pairing::align(&mine, &theirs);
    Aligned {
        pairs: found
            .pairs
            .iter()
            .filter_map(|(i, j)| paragraphs.get(*i).cloned().zip(reference.get(*j).cloned()))
            .collect(),
        missing: found.missing,
        extra: found.extra,
    }
}

/// The paragraph style and direct formatting of each `(paragraph, reference)`: the
/// paragraph's in `block` of `mine`, the reference's in `source` of `theirs`.
pub(crate) fn paragraphs(
    block: &Block<'_>,
    mine: &Side<'_>,
    source: &Block<'_>,
    theirs: &Side<'_>,
    pairs: &[(Para, Para)],
) -> Vec<Difference> {
    let mut groups: Groups<(&'static str, Value, Value), String> = Groups::new();
    for (paragraph, reference) in pairs {
        let Some(id) = block.paragraph_id(&paragraph.1) else {
            continue;
        };
        let values = paragraph_values(block, mine, paragraph);
        let reference_values = paragraph_values(source, theirs, reference);
        for ((name, value), (_, reference)) in values.into_iter().zip(reference_values) {
            if value != reference {
                groups.add((name, value, reference), id.clone());
            }
        }
    }
    groups
        .into_iter()
        .map(|((name, value, reference), ids)| {
            let fix = paragraph_fix(mine, name, &reference, &ids);
            difference(name, Some(("paragraphs", json!(ids))), value, reference, fix)
        })
        .collect()
}

/// The paragraph's style, by name: the one it states, else the default.
pub(crate) fn paragraph_style(side: &Side<'_>, paragraph: &Element) -> Option<String> {
    match find_path(paragraph, &["pPr", "pStyle"]) {
        Some(style) => side.style_name(style.attribute(Some(W), "val")),
        None => side.style_name(side.default_style()),
    }
}

fn paragraph_values(block: &Block<'_>, side: &Side<'_>, paragraph: &Para) -> Vec<(&'static str, Value)> {
    let (element, below) = paragraph;
    let path: Vec<u32> = block.path.iter().chain(below).copied().collect();
    let cell = side.cell(block.part, element, &path);
    let properties = element.find(Some(W), "pPr");
    let declared: Vec<&str> = PARAGRAPH
        .iter()
        .filter(|(_, tag, attributes)| {
            properties.and_then(|p| p.find(Some(W), tag)).is_some_and(|node| {
                attributes.is_empty() || attributes.iter().any(|a| node.attribute(Some(W), a).is_some())
            })
        })
        .map(|(name, _, _)| *name)
        .collect();
    let effective = (!declared.is_empty()).then(|| side.formatting().paragraph(element, cell.as_ref()));
    let mut out = vec![("paragraph_style", json!(paragraph_style(side, element)))];
    for (name, _, _) in PARAGRAPH {
        let value = match &effective {
            Some(effective) if declared.contains(&name) => paragraph_value(effective, name),
            _ => Value::Null,
        };
        out.push((name, value));
    }
    out.extend(run_values(side, element, cell.as_ref()));
    out
}

fn paragraph_value(effective: &EffectiveParagraph, name: &str) -> Value {
    match name {
        "indent_first" => json!(round2(effective.first_line - effective.hanging)),
        "line_spacing" => match effective.line_spacing {
            None => Value::Null,
            Some(spacing) if effective.line_rule == "auto" => json!(round2(spacing)),
            Some(spacing) => json!(format!("{} {}pt", effective.line_rule, python_g(round2(spacing)))),
        },
        "alignment" => json!(effective.alignment),
        "space_before" => json!(round2(effective.space_before)),
        "space_after" => json!(round2(effective.space_after)),
        "indent_left" => json!(round2(effective.indent_left)),
        "indent_right" => json!(round2(effective.indent_right)),
        "keep_with_next" => json!(effective.keep_with_next),
        "keep_lines" => json!(effective.keep_together),
        "page_break_before" => json!(effective.page_break_before),
        _ => Value::Null,
    }
}

/// Per run property, the value most of the paragraph's text has (`null`: nothing stated,
/// the style's then; a tie goes to `null`, then to the first value met).
fn run_values(
    side: &Side<'_>,
    paragraph: &std::sync::Arc<Element>,
    cell: Option<&ooxml_word_model::docx::formatting::Cell>,
) -> Vec<(&'static str, Value)> {
    let mut counts: Vec<Vec<(Value, usize)>> = vec![Vec::new(); RUN.len()];
    for run in text_runs(paragraph, View::Current) {
        let Some(element) = at_path(paragraph, &run.path) else {
            continue;
        };
        let properties = element.find(Some(W), "rPr");
        let declared = |tags: &[&str]| properties.is_some_and(|p| tags.iter().any(|t| p.find(Some(W), t).is_some()));
        let stated = RUN.iter().any(|(_, tags)| declared(tags));
        let effective = stated.then(|| side.formatting().run(paragraph, Some(element), run.first, cell));
        for ((name, tags), found) in RUN.iter().zip(counts.iter_mut()) {
            let value = match &effective {
                Some(effective) if declared(tags) => run_value(effective, name),
                _ => Value::Null,
            };
            match found.iter_mut().find(|(v, _)| *v == value) {
                Some((_, count)) => *count += run.text,
                None => found.push((value, run.text)),
            }
        }
    }
    RUN.iter()
        .zip(counts)
        .map(|((name, _), found)| {
            let mut best: Option<&(Value, usize)> = None;
            for entry in &found {
                let key = |e: &(Value, usize)| (e.1, e.0.is_null());
                if best.is_none_or(|b| key(entry) > key(b)) {
                    best = Some(entry);
                }
            }
            (*name, best.map_or(Value::Null, |(v, _)| v.clone()))
        })
        .collect()
}

fn run_value(effective: &EffectiveRun, name: &str) -> Value {
    match name {
        "bold" => json!(effective.bold),
        "italic" => json!(effective.italic),
        "underline" => json!(effective
            .underline
            .as_deref()
            .filter(|u| *u != "none")
            .unwrap_or("none")),
        "strike" => json!(effective.strike),
        "size" => json!(round2(effective.size)),
        "font" => json!(effective.font),
        "color" => json!(effective.color),
        "highlight" => json!(effective.highlight),
        _ => Value::Null,
    }
}

fn paragraph_fix(side: &Side<'_>, name: &str, reference: &Value, paragraphs: &[String]) -> Value {
    let mut call = Map::new();
    call.insert("targets".into(), json!(paragraphs));
    match (name, reference) {
        ("paragraph_style", Value::String(style)) if side.has_style(style, "paragraph") => {
            call.insert("paragraph_style".into(), reference.clone());
        }
        ("paragraph_style", _) => return Value::Null,
        (_, Value::Null) => {
            call.insert("clear".into(), json!([name]));
        }
        ("line_spacing", value) if !value.is_number() => return Value::Null,
        ("underline", value) => match value.as_str() {
            Some(u @ ("single" | "none")) => {
                call.insert("underline".into(), json!(u == "single"));
            }
            _ => return Value::Null,
        },
        ("alignment", value) => {
            let word = match value.as_str() {
                Some("both") => "justify",
                Some("start") => "left",
                Some("end") => "right",
                Some(other) => other,
                None => return Value::Null,
            };
            if !matches!(word, "left" | "center" | "right" | "justify") {
                return Value::Null;
            }
            call.insert("alignment".into(), json!(word));
        }
        ("color", value) => match value.as_str() {
            Some(color) if color.chars().count() == 6 => {
                call.insert("color".into(), value.clone());
            }
            _ => return Value::Null,
        },
        (name, value) => {
            call.insert(name.into(), value.clone());
        }
    }
    fix("word_format", vec![Value::Object(call)])
}
