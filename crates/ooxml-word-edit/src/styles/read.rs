//! Owns: what a style states, read from a version as docx-agent's `describe_style` and
//! `Styles` read it: its own run and paragraph properties (`read_run`, `read_paragraph`),
//! its tab stops (`read_tabs`) and those it inherits (`inherited_tabs`), a table style's
//! regions (`read_regions`, `needed_looks`), the list level it numbers at (`style_link`),
//! where it is used (`usage`), and the table style new tables get (`table_default`).
//!
//! Must not: change anything, or write a tool's JSON (typed values only).

use super::regions::{border_words, BORDER_SIDES, KINDS};
use super::sheet::STYLE_REFERENCES;
use super::tabs::TAB_LEADERS;
use crate::names::W;
use ooxml_word_model::docx::{style_id, Docx};
use ooxml_word_model::xmlread::py_int;
use ooxml_xml::Element;
use std::sync::Arc;

/// A property as a style declares it.
#[derive(Debug, Clone, PartialEq)]
pub enum Declared {
    /// On or off.
    Flag(bool),
    /// Points, a size, a multiple.
    Number(f64),
    /// A name or value.
    Text(String),
    /// A line spacing with its rule (`exact`, `at_least`) and points.
    Rule(&'static str, f64),
}

/// The run properties docx-agent's `describe_style` reports, in its (sorted) order.
pub const RUN_NAMES: &[&str] = &[
    "bold",
    "caps",
    "color",
    "double_strike",
    "font",
    "highlight",
    "italic",
    "size",
    "small_caps",
    "strike",
    "underline",
    "vertical_align",
];

/// The paragraph properties a paragraph style's description reports, in order.
pub const PARAGRAPH_NAMES: &[&str] = &[
    "alignment",
    "space_before",
    "space_after",
    "line_spacing",
    "indent_left",
    "indent_right",
    "first_line",
    "hanging",
    "keep_with_next",
    "keep_together",
    "page_break_before",
];

fn child<'e>(element: Option<&'e Element>, local: &str) -> Option<&'e Arc<Element>> {
    element?.find(Some(W), local)
}

fn attr<'e>(element: &'e Element, local: &str) -> Option<&'e str> {
    element.attribute(Some(W), local)
}

/// A toggle's value (`_on`).
fn toggle(node: Option<&Arc<Element>>) -> Option<Declared> {
    let node = node?;
    let value = attr(node, "val").unwrap_or("true").to_lowercase();
    Some(Declared::Flag(!matches!(value.as_str(), "0" | "false" | "off")))
}

/// A run property as `properties` (a `w:rPr`) declares it (`read_run`).
#[must_use]
pub fn read_run(properties: Option<&Element>, name: &str) -> Option<Declared> {
    let properties = properties?;
    let text = |local: &str| {
        child(Some(properties), local)
            .and_then(|n| attr(n, "val"))
            .map(|v| Declared::Text(v.to_owned()))
    };
    match name {
        "bold" => toggle(child(Some(properties), "b")),
        "italic" => toggle(child(Some(properties), "i")),
        "strike" => toggle(child(Some(properties), "strike")),
        "double_strike" => toggle(child(Some(properties), "dstrike")),
        "caps" => toggle(child(Some(properties), "caps")),
        "small_caps" => toggle(child(Some(properties), "smallCaps")),
        "underline" => {
            let node = child(Some(properties), "u")?;
            Some(match attr(node, "val").filter(|v| !v.is_empty()).unwrap_or("single") {
                "none" => Declared::Flag(false),
                "single" => Declared::Flag(true),
                other => Declared::Text(other.to_owned()),
            })
        }
        "size" => child(Some(properties), "sz")
            .and_then(|n| attr(n, "val"))
            .and_then(py_int)
            .map(|v| Declared::Number(v as f64 / 2.0)),
        "font" => {
            let node = child(Some(properties), "rFonts")?;
            attr(node, "ascii")
                .filter(|v| !v.is_empty())
                .or_else(|| attr(node, "hAnsi"))
                .map(|v| Declared::Text(v.to_owned()))
        }
        "color" => {
            let node = child(Some(properties), "color")?;
            attr(node, "themeColor")
                .filter(|v| !v.is_empty())
                .or_else(|| attr(node, "val"))
                .map(|v| Declared::Text(v.to_owned()))
        }
        "highlight" => text("highlight"),
        "vertical_align" => text("vertAlign"),
        _ => None,
    }
}

fn points(value: &str) -> Option<f64> {
    py_int(value).map(|v| v as f64 / 20.0)
}

/// A paragraph property as `properties` (a `w:pPr`) declares it (`read_paragraph`).
#[must_use]
pub fn read_paragraph(properties: Option<&Element>, name: &str) -> Option<Declared> {
    let properties = properties?;
    match name {
        "keep_with_next" => toggle(child(Some(properties), "keepNext")),
        "keep_together" => toggle(child(Some(properties), "keepLines")),
        "page_break_before" => toggle(child(Some(properties), "pageBreakBefore")),
        "widow_control" => toggle(child(Some(properties), "widowControl")),
        "alignment" => {
            let value = attr(child(Some(properties), "jc")?, "val")?;
            let named = match value {
                "start" => "left",
                "end" => "right",
                "both" => "justify",
                other => other,
            };
            Some(Declared::Text(named.to_owned()))
        }
        "indent_left" | "indent_right" | "first_line" | "hanging" => {
            let node = child(Some(properties), "ind")?;
            let names: &[&str] = match name {
                "indent_left" => &["left", "start"],
                "indent_right" => &["right", "end"],
                "first_line" => &["firstLine"],
                _ => &["hanging"],
            };
            let given = names.iter().find_map(|a| attr(node, a))?;
            points(given).map(Declared::Number)
        }
        "space_before" | "space_after" => {
            let node = child(Some(properties), "spacing")?;
            let local = if name == "space_before" { "before" } else { "after" };
            attr(node, local).and_then(points).map(Declared::Number)
        }
        "line_spacing" => {
            let node = child(Some(properties), "spacing")?;
            let line = py_int(attr(node, "line")?)? as f64;
            Some(match attr(node, "lineRule").unwrap_or("auto") {
                "auto" => Declared::Number(line / 240.0),
                "exact" => Declared::Rule("exact", line / 20.0),
                _ => Declared::Rule("at_least", line / 20.0),
            })
        }
        _ => None,
    }
}

/// What a style declares itself: its run properties, and a paragraph style's paragraph
/// properties (`keep_together` reported as `keep_lines`), in `describe_style`'s order.
#[must_use]
pub fn declared(style: &Element, paragraph_style: bool) -> Vec<(&'static str, Declared)> {
    let run = child(Some(style), "rPr").map(|r| &**r);
    let mut out: Vec<(&'static str, Declared)> = RUN_NAMES
        .iter()
        .filter_map(|n| read_run(run, n).map(|v| (*n, v)))
        .collect();
    if paragraph_style {
        let paragraph = child(Some(style), "pPr").map(|p| &**p);
        for name in PARAGRAPH_NAMES {
            if let Some(value) = read_paragraph(paragraph, name) {
                out.push((if *name == "keep_together" { "keep_lines" } else { name }, value));
            }
        }
    }
    out
}

/// One tab stop as a style states it: points, alignment, leader.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadStop {
    /// Points.
    pub position: f64,
    /// Its alignment (`start` read as left, `end` as right).
    pub alignment: String,
    /// Its leader, when not none.
    pub leader: Option<String>,
}

/// A style's own tab stops (`read_tabs`); `None` for none.
#[must_use]
pub fn read_tabs(style: &Element) -> Option<Vec<ReadStop>> {
    let tabs: Vec<&Arc<Element>> = child(child(Some(style), "pPr").map(|p| &**p), "tabs")
        .map(|t| t.child_elements().filter(|c| c.name().is(Some(W), "tab")).collect())
        .unwrap_or_default();
    if tabs.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for tab in tabs {
        let Some(position) = attr(tab, "pos").and_then(py_int) else {
            continue;
        };
        let value = attr(tab, "val").filter(|v| !v.is_empty()).unwrap_or("left");
        let alignment = match value {
            "start" => "left",
            "end" => "right",
            other => other,
        };
        let leader = attr(tab, "leader").filter(|l| *l != "none").map(|l| {
            TAB_LEADERS
                .iter()
                .find(|(_, x)| *x == l)
                .map_or(l, |(n, _)| *n)
                .to_owned()
        });
        out.push(ReadStop {
            position: position as f64 / 20.0,
            alignment: alignment.to_owned(),
            leader,
        });
    }
    Some(out)
}

fn based_on(style: &Element) -> Option<&str> {
    child(Some(style), "basedOn").and_then(|b| attr(b, "val"))
}

/// The regions as the tools name them and their `w:tblStylePr` types (`whole_table`: the
/// style's own).
pub const REGIONS: &[(&str, Option<&str>)] = KINDS;

/// The look options (`word_format_table` `look`) each region shows under.
pub const LOOK_OF: &[(&str, &[&str])] = &[
    ("header_row", &["header_row"]),
    ("total_row", &["total_row"]),
    ("first_column", &["first_column"]),
    ("last_column", &["last_column"]),
    ("banded_rows", &["banded_rows"]),
    ("even_banded_rows", &["banded_rows"]),
    ("banded_columns", &["banded_columns"]),
    ("even_banded_columns", &["banded_columns"]),
    ("top_left_cell", &["header_row", "first_column"]),
    ("top_right_cell", &["header_row", "last_column"]),
    ("bottom_left_cell", &["total_row", "first_column"]),
    ("bottom_right_cell", &["total_row", "last_column"]),
];

/// A region's formatting as the style states it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Region {
    /// Text colour.
    pub color: Option<String>,
    /// Bold.
    pub bold: Option<bool>,
    /// Italic.
    pub italic: Option<bool>,
    /// Text size.
    pub size: Option<f64>,
    /// Alignment.
    pub alignment: Option<String>,
    /// Fill.
    pub shading: Option<String>,
    /// Borders by side, in words.
    pub borders: Vec<(&'static str, String)>,
    /// Rows or columns per band.
    pub band_size: Option<i64>,
}

impl Region {
    fn is_empty(&self) -> bool {
        *self == Region::default()
    }
}

fn read_part(part: &Element, whole: bool) -> Region {
    let run = child(Some(part), "rPr").map(|r| &**r);
    let flag = |name: &str| match read_run(run, name) {
        Some(Declared::Flag(on)) => Some(on),
        _ => None,
    };
    let holder = if whole {
        child(child(Some(part), "tblPr").map(|t| &**t), "tblBorders")
    } else {
        child(child(Some(part), "tcPr").map(|t| &**t), "tcBorders")
    };
    let borders = holder
        .map(|h| {
            BORDER_SIDES
                .iter()
                .filter_map(|(side, local)| child(Some(h), local).map(|b| (*side, border_words(b))))
                .collect()
        })
        .unwrap_or_default();
    Region {
        color: match read_run(run, "color") {
            Some(Declared::Text(t)) => Some(t),
            _ => None,
        },
        bold: flag("bold"),
        italic: flag("italic"),
        size: match read_run(run, "size") {
            Some(Declared::Number(n)) => Some(n),
            _ => None,
        },
        alignment: match read_paragraph(child(Some(part), "pPr").map(|p| &**p), "alignment") {
            Some(Declared::Text(t)) => Some(t),
            _ => None,
        },
        shading: super::shading::read_shading(child(Some(part), "tcPr").map(|c| &**c)),
        borders,
        band_size: None,
    }
}

/// A table style's regions as it states them (`read_regions`): `(region, formatting)` --
/// the whole table from its own properties, a `w:tblStylePr wholeTable` as
/// `whole_table_part` -- with the band size on the banded ones where stated.
#[must_use]
pub fn read_regions(style: &Element) -> Vec<(String, Region)> {
    let mut out: Vec<(String, Region)> = Vec::new();
    let whole = read_part(style, true);
    if !whole.is_empty() {
        out.push(("whole_table".to_owned(), whole));
    }
    let table = child(Some(style), "tblPr").map(|t| &**t);
    let mut sizes: Vec<(&str, i64)> = Vec::new();
    for (local, names) in [
        ("tblStyleRowBandSize", ["banded_rows", "even_banded_rows"]),
        ("tblStyleColBandSize", ["banded_columns", "even_banded_columns"]),
    ] {
        if let Some(size) = child(table, local).and_then(|n| attr(n, "val")).and_then(py_int) {
            sizes.extend(names.iter().map(|n| (*n, size)));
        }
    }
    for part in style.child_elements().filter(|c| c.name().is(Some(W), "tblStylePr")) {
        let kind = attr(part, "type");
        let values = read_part(part, false);
        match KINDS.iter().find(|(_, k)| k.is_some() && *k == kind) {
            Some((name, _)) => {
                if !values.is_empty() {
                    match out.iter_mut().find(|(n, _)| n == name) {
                        Some(slot) => slot.1 = values,
                        None => out.push(((*name).to_owned(), values)),
                    }
                }
            }
            None if kind == Some("wholeTable") && !values.is_empty() => {
                match out.iter_mut().find(|(n, _)| n == "whole_table_part") {
                    Some(slot) => slot.1 = values,
                    None => out.push(("whole_table_part".to_owned(), values)),
                }
            }
            None => {}
        }
    }
    for (name, size) in sizes {
        if let Some((_, region)) = out.iter_mut().find(|(n, _)| n == name) {
            region.band_size = Some(size);
        }
    }
    out
}

/// The look options the regions named need, in order (`needed_looks`).
#[must_use]
pub fn looks_needed<'a>(regions: impl IntoIterator<Item = &'a str>) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for region in regions {
        for option in LOOK_OF.iter().find(|(r, _)| *r == region).map_or(&[][..], |(_, o)| o) {
            if !out.contains(option) {
                out.push(option);
            }
        }
    }
    out
}

/// How a paragraph style numbers its paragraphs (`style_link`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleLink {
    /// Its own `w:numId` 0 stops numbering.
    None,
    /// It numbers at `level` of `list`; `from` the style in its chain holding the
    /// `w:numPr` when not its own.
    Level {
        /// The list's number.
        list: i64,
        /// The level.
        level: i64,
        /// Whether the `w:numId` is its own.
        own: bool,
        /// The style holding it, when not its own.
        from: Option<String>,
    },
}

/// The style element of `id` as docx-agent's dict of styles holds it: the last one.
fn last_node<'d>(docx: &'d Docx, id: &str) -> Option<&'d Arc<Element>> {
    docx.styles().iter().rev().find(|s| style_id(s) == id)
}

fn own_num_id(style: &Element) -> Option<&Arc<Element>> {
    child(
        child(child(Some(style), "pPr").map(|p| &**p), "numPr").map(|n| &**n),
        "numId",
    )
}

/// How style `id` numbers its paragraphs, `None` when it numbers nothing.
#[must_use]
pub fn style_link(docx: &Docx, id: &str) -> Option<StyleLink> {
    let own = last_node(docx, id).and_then(|n| own_num_id(n));
    if own.and_then(|o| attr(o, "val")).and_then(py_int) == Some(0) {
        return Some(StyleLink::None);
    }
    let (list, level) = crate::lists::style_list_of(docx, Some(id))?;
    let mut holder = Some(id.to_owned());
    let mut seen: Vec<String> = Vec::new();
    while let Some(current) = holder.clone().filter(|h| !h.is_empty() && !seen.contains(h)) {
        let Some(node) = last_node(docx, &current) else { break };
        seen.push(current);
        if own_num_id(node).is_some() {
            break;
        }
        holder = based_on(node).map(str::to_owned);
    }
    let own = holder.as_deref() == Some(id);
    Some(StyleLink::Level {
        list,
        level,
        own,
        from: if own { None } else { holder },
    })
}

/// Where a style is used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// References from paragraphs, runs, tables and list levels (every part but the style
    /// sheet).
    pub content: usize,
    /// References from other styles.
    pub styles: usize,
}

/// The tags content names a style by (`_content_references`').
pub(crate) const CONTENT_REFERENCES: &[&str] = &["pStyle", "rStyle", "tblStyle", "numStyleLink", "styleLink"];

/// The parts content references are looked for in: every `word/*.xml` but the style sheet.
pub(crate) fn content_parts(docx: &Docx, styles: Option<&str>) -> Vec<String> {
    docx.package
        .part_names()
        .into_iter()
        .filter(|p| Some(p.as_str()) != styles && p.starts_with("word/") && p.ends_with(".xml"))
        .collect()
}

fn count_in(element: &Element, locals: &[&str], id: &str) -> usize {
    let own = usize::from(
        element.name().namespace() == Some(W)
            && locals.contains(&element.name().local())
            && attr(element, "val") == Some(id),
    );
    own + element.child_elements().map(|c| count_in(c, locals, id)).sum::<usize>()
}

/// Where style `id` is used (`Styles.usage`).
///
/// # Errors
/// When a part is not XML.
pub fn usage(docx: &Docx, id: &str) -> crate::Result<Usage> {
    let styles = ooxml_word_model::parts::related(&docx.package, &docx.main, super::sheet::REL_STYLES)?;
    let mut content = 0;
    for part in content_parts(docx, styles.as_deref()) {
        if let Ok(Some(tree)) = docx.package.tree(&part) {
            content += count_in(tree.root(), CONTENT_REFERENCES, id);
        }
    }
    let styles = docx
        .styles()
        .iter()
        .filter(|s| style_id(s) != id)
        .map(|s| count_in(s, STYLE_REFERENCES, id))
        .sum();
    Ok(Usage { content, styles })
}

/// The table style new tables get: the one the settings' `w:defaultTableStyle` names,
/// when the document has it (`Styles.table_default`).
#[must_use]
pub fn table_default(docx: &Docx) -> Option<Arc<Element>> {
    let settings = ooxml_word_model::parts::related(&docx.package, &docx.main, ooxml_word_model::parts::REL_SETTINGS)
        .ok()
        .flatten()?;
    let tree = docx.package.tree(&settings).ok().flatten()?;
    let named = child(Some(tree.root()), "defaultTableStyle")
        .and_then(|n| attr(n, "val"))
        .filter(|v| !v.is_empty())?
        .to_owned();
    docx.styles()
        .iter()
        .find(|s| style_id(s) == named && ooxml_word_model::docx::style_kind(s) == "table")
        .cloned()
}

/// How many times content names each style id (every part but the sheet), read once.
#[must_use]
pub fn content_uses(docx: &Docx) -> std::collections::HashMap<String, usize> {
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
    let styles = ooxml_word_model::parts::related(&docx.package, &docx.main, super::sheet::REL_STYLES)
        .ok()
        .flatten();
    let mut out = std::collections::HashMap::new();
    for part in content_parts(docx, styles.as_deref()) {
        if let Ok(Some(tree)) = docx.package.tree(&part) {
            walk(tree.root(), &mut out);
        }
    }
    out
}

/// The style `name` names among those of `kind` (any kind for `None`), as docx-agent's
/// `Styles.find` finds it.
#[must_use]
pub fn find_style(docx: &Docx, name: &str, kind: Option<&str>) -> Option<Arc<Element>> {
    super::sheet::find_in(docx, name, kind)
}
