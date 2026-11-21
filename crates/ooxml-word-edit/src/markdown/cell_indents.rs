//! Owns: insert_markdown's check of the tables it wrote (round 21, T4; docx-agent's
//! `cell_indent_warnings`): a cell paragraph takes its style as it is -- no direct indent
//! is written to undo what the style inherits -- and where the style's effective indents
//! run its text past the cell's edge (an indent more negative than the cell's margin), or
//! leave it no width in its grid share, a warning names the style, the indent and the style
//! that declared it, once per table, style and problem.
//!
//! Must not: write anything; read anything but the tables given and the styles they use.

use crate::names::W;
use ooxml_word_model::docx::cells::{model_path, TableFacts};
use ooxml_word_model::docx::formatting::Formatting;
use ooxml_word_model::resolve::{resolve_table, Origins};
use ooxml_word_model::styles::{StyleKind, StyleSheet};
use ooxml_xml::Element;
use std::collections::HashSet;
use std::sync::Arc;

/// A cell's side margin, pt, where the table cannot be resolved: Word's Normal Table's.
const DEFAULT_CELL_MARGIN: f64 = 5.4;

/// The warnings of `tables` (each a `w:tbl` the edit wrote, with its grid shares in twips).
pub(crate) fn cell_indent_warnings(formatting: &Formatting, tables: &[(Arc<Element>, Vec<i64>)]) -> Vec<String> {
    let mut out = Vec::new();
    for (table, shares) in tables {
        table_warnings(formatting, table, shares, &mut out);
    }
    out
}

/// One problem of a paragraph's indents: which (`indent_left`, `indent_right`, `width`)
/// and what the warning says of it.
type Problem = (&'static str, String);

fn table_warnings(formatting: &Formatting, table: &Arc<Element>, shares: &[i64], out: &mut Vec<String>) {
    let facts = TableFacts::read(formatting, table);
    let resolved = facts
        .tables()
        .first()
        .map(|model| resolve_table(&formatting.sources(), model));
    let sheet = formatting.sources().styles;
    let mut seen: HashSet<(Option<String>, &'static str)> = HashSet::new();
    for (r, (row_at, row)) in children(table, "tr").enumerate() {
        for (column, (cell_at, cell)) in children(row, "tc").enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let width = shares.get(column).map(|share| *share as f64 / 20.0);
            #[allow(clippy::cast_precision_loss)]
            let (left_margin, right_margin) = resolved
                .as_ref()
                .and_then(|t| t.rows.get(r)?.cells.get(column).cloned())
                .map_or((DEFAULT_CELL_MARGIN, DEFAULT_CELL_MARGIN), |c| {
                    (c.margins[1] as f64 / 20.0, c.margins[3] as f64 / 20.0)
                });
            let mut paragraphs = Vec::new();
            paragraphs_below(cell, vec![row_at, cell_at], &mut paragraphs);
            for (place, paragraph) in paragraphs {
                let at = model_path(table, &place).and_then(|path| facts.cell(&path).cloned());
                let (look, origins) = formatting.paragraph_traced(paragraph, at.as_ref());
                let first = (look.first_line - look.hanging).max(0.0);
                let mut problems: Vec<Problem> = Vec::new();
                for (side, value, margin, key) in [
                    ("indent_left", look.indent_left, left_margin, "ind.left"),
                    ("indent_right", look.indent_right, right_margin, "ind.right"),
                ] {
                    if value < -margin {
                        let source = declared_in(sheet, &origins, key);
                        problems.push((
                            side,
                            format!(
                                "{side} {} pt{source} runs its text {} pt past the cell's edge",
                                py_g(value),
                                py_g(-value - margin)
                            ),
                        ));
                    }
                }
                if let Some(width) = width {
                    let room = width - left_margin - right_margin;
                    if room - look.indent_left - look.indent_right - first <= 0.0 {
                        problems.push((
                            "width",
                            format!(
                                "indent_left {} pt and indent_right {} pt leave its text no width in a {} pt cell",
                                py_g(look.indent_left),
                                py_g(look.indent_right),
                                py_g(width)
                            ),
                        ));
                    }
                }
                let style = paragraph_style(paragraph);
                for (key, text) in problems {
                    if seen.insert((style.clone(), key)) {
                        out.push(format!(
                            "table cells in style {}: the style's {text}; give the style indents that fit a cell, \
                             or map table_cell to another style",
                            py_repr(&style_name(sheet, style.as_deref()))
                        ));
                    }
                }
            }
        }
    }
}

/// The `w:<local>` children of `parent`, with their child indexes.
fn children<'e>(parent: &'e Element, local: &'static str) -> impl Iterator<Item = (u32, &'e Arc<Element>)> {
    parent.children().iter().enumerate().filter_map(move |(k, node)| {
        let element = node.as_element().filter(|e| e.name().is(Some(W), local))?;
        Some((u32::try_from(k).ok()?, element))
    })
}

/// Every `w:p` at or below `element` (document order), each with its place below the table.
fn paragraphs_below<'e>(element: &'e Element, place: Vec<u32>, out: &mut Vec<(Vec<u32>, &'e Element)>) {
    for (k, node) in element.children().iter().enumerate() {
        let (Some(child), Ok(k)) = (node.as_element(), u32::try_from(k)) else {
            continue;
        };
        let mut at = place.clone();
        at.push(k);
        if child.name().is(Some(W), "p") {
            out.push((at.clone(), child));
        }
        paragraphs_below(child, at, out);
    }
}

/// The paragraph's `w:pStyle`.
fn paragraph_style(paragraph: &Element) -> Option<String> {
    let properties = paragraph.find(Some(W), "pPr")?;
    properties
        .find(Some(W), "pStyle")?
        .attribute(Some(W), "val")
        .map(str::to_owned)
}

/// `` (from <style>)`` where the value of `key` was declared in a style (docx-agent's
/// `_declared_in`).
fn declared_in(sheet: Option<&StyleSheet>, origins: &Origins, key: &str) -> String {
    let declared = sheet.and_then(|sheet| origins.of(key)?.declared_in(sheet));
    declared.map_or_else(String::new, |style| {
        format!(" (from {})", style_name(sheet, Some(&style.style_id)))
    })
}

/// A style's name as docx-agent's `_style_name` gives it: its `w:name`, else its id; for no
/// style, the default paragraph style's name (else `Normal`).
fn style_name(sheet: Option<&StyleSheet>, id: Option<&str>) -> String {
    let named = |style: &ooxml_word_model::styles::Style| style.name.as_deref().map(str::to_owned);
    match id {
        None => sheet
            .and_then(|s| s.default_of(StyleKind::Paragraph))
            .and_then(|s| named(s))
            .unwrap_or_else(|| "Normal".to_owned()),
        Some(id) => sheet
            .and_then(|s| s.get(id))
            .and_then(|s| named(s))
            .unwrap_or_else(|| id.to_owned()),
    }
}

/// `text` as Python's `repr` of a string writes it (single quotes unless it holds one and
/// no double quote).
fn py_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `value` as Python's `f"{value:g}"` writes it: six significant digits, no trailing zeros
/// (points here are far from the exponent form's bounds).
fn py_g(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    #[allow(clippy::cast_possible_truncation)]
    let exponent = value.abs().log10().floor() as i32;
    let decimals = usize::try_from(5 - exponent).unwrap_or(0);
    let fixed = format!("{value:.decimals$}");
    let text = if fixed.contains('.') {
        fixed.trim_end_matches('0').trim_end_matches('.')
    } else {
        &fixed
    };
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{py_g, py_repr};

    #[test]
    fn numbers_as_python_formats_them() {
        assert_eq!(py_g(-170.0), "-170");
        assert_eq!(py_g(170.0 - 5.4), "164.6");
        assert_eq!(py_g(5.4), "5.4");
        assert_eq!(py_g(225.65), "225.65");
        assert_eq!(py_g(0.05), "0.05");
        assert_eq!(py_g(0.0), "0");
    }

    #[test]
    fn names_as_python_quotes_them() {
        assert_eq!(py_repr("Cell text"), "'Cell text'");
        assert_eq!(py_repr("Bob's"), "\"Bob's\"");
    }
}
