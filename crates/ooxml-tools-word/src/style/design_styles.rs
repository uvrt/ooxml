//! Owns: `word_design_facts`' `styles` and `declared_styles` (docx-agent's
//! `edit/design.py`, "styles" and "declared styles"): every paragraph style in use, per
//! role, with its usage and its look after inheritance (in a cell, with the table style the
//! cells hold most); and every paragraph, character and table style the document declares,
//! used or not, its look given as what differs from the default paragraph style's.
//!
//! Must not: change a document, or read direct formatting (a look is a style's).

use super::design_look::Design;
use super::design_tables::{element_at, places, Tally};
use ooxml_base::Cancel;
use ooxml_tools::ToolError;
use ooxml_word_model::docx::formatting::Cell;
use ooxml_word_model::docx::{child_val, find_path, on, style_id, style_kind, style_name, Docx};
use ooxml_word_model::ids::W;
use ooxml_word_model::markdown::read::Reader;
use ooxml_word_model::model::Look;
use ooxml_word_model::value::key;
use ooxml_word_model::view::View;
use ooxml_xml::Element;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Roles in the order they are listed.
const ROLES: [&str; 20] = [
    "title",
    "subtitle",
    "heading 1",
    "heading 2",
    "heading 3",
    "heading 4",
    "heading 5",
    "heading 6",
    "heading 7",
    "heading 8",
    "heading 9",
    "body",
    "list",
    "caption",
    "table",
    "toc",
    "header",
    "footer",
    "footnote",
    "endnote",
];

/// `w:tblStylePr` types and the style options that turn them on.
const PART_OPTIONS: [&str; 4] = ["firstRow", "lastRow", "firstCol", "lastCol"];

/// Paragraph styles by id, the one applied and its chain of names.
struct Names<'d> {
    styles: HashMap<&'d str, &'d Arc<Element>>,
    default: Option<&'d Arc<Element>>,
}

impl<'d> Names<'d> {
    fn new(docx: &'d Docx) -> Self {
        let styles = docx
            .styles()
            .iter()
            .filter(|s| style_kind(s) == "paragraph")
            .map(|s| (style_id(s), s))
            .collect();
        Names {
            styles,
            default: docx.default_style("paragraph"),
        }
    }

    /// The style applied: the one named where it is a paragraph style, else the default.
    fn applied(&self, id: Option<&str>) -> Option<&'d Arc<Element>> {
        id.filter(|i| !i.is_empty())
            .and_then(|i| self.styles.get(i).copied())
            .or(self.default)
    }

    /// The names (lower case) of a style and those it is based on.
    fn chain(&self, style: Option<&'d Arc<Element>>) -> Vec<String> {
        let mut names = Vec::new();
        let mut seen = HashSet::new();
        let mut current = style;
        while let Some(s) = current {
            if !seen.insert(style_id(s)) {
                break;
            }
            names.push(style_name(s).unwrap_or("").to_lowercase());
            current = child_val(s, "basedOn").and_then(|b| self.styles.get(b).copied());
        }
        names
    }
}

/// Whether a paragraph holds no text and nothing drawn: a spacer, or a section break's.
pub(crate) fn empty(node: &Element) -> bool {
    let mut text = String::new();
    let mut drawn = false;
    let mut stack = vec![node];
    while let Some(e) = stack.pop() {
        for child in e.child_elements() {
            if child.name().namespace() == Some(W) {
                match child.name().local() {
                    "t" => text.push_str(&child.text_content()),
                    "drawing" | "pict" | "object" => drawn = true,
                    _ => {}
                }
            }
            stack.push(child);
        }
    }
    text.trim().is_empty() && !drawn
}

/// The innermost table holding the element at `path` of a part.
fn table_above(root: &Arc<Element>, path: &[u32]) -> Option<Arc<Element>> {
    (1..path.len())
        .rev()
        .filter_map(|n| element_at(root, path.get(..n)?))
        .find(|e| e.name().is(Some(W), "tbl"))
}

/// One style in one role.
struct Usage {
    role: &'static str,
    style: Option<String>,
    count: usize,
    example: String,
    empty: usize,
    tables: Tally<String>,
}

/// A body paragraph's role (`None`: a story whose paragraphs are not counted).
fn role(
    design: &Design<'_>,
    reader: &mut Reader<'_>,
    names: &Names<'_>,
    at: (&str, &Element, bool),
) -> Option<&'static str> {
    let (story, node, in_table) = at;
    if story.starts_with("header") {
        return Some("header");
    }
    if story.starts_with("footer") {
        return Some("footer");
    }
    match story {
        "footnotes" => return Some("footnote"),
        "endnotes" => return Some("endnote"),
        "body" => {}
        _ => return None,
    }
    if in_table {
        return Some("table");
    }
    if let Some(level) = reader.heading_level(node) {
        return ROLES.get(usize::try_from(level + 1).unwrap_or(0)).copied();
    }
    let declared = find_path(node, &["pPr", "pStyle"]).and_then(|s| s.attribute(Some(W), "val"));
    let chain = names.chain(names.applied(declared));
    match chain.first().map(String::as_str) {
        Some("title") => return Some("title"),
        Some("subtitle") => return Some("subtitle"),
        _ => {}
    }
    if chain.iter().any(|n| n == "caption") {
        return Some("caption");
    }
    if chain
        .first()
        .is_some_and(|n| n.starts_with("toc ") || n == "toc heading")
    {
        return Some("toc");
    }
    if let Some(direct) = find_path(node, &["pPr", "numPr"]) {
        let listed = find_path(direct, &["numId"])
            .and_then(|n| n.attribute(Some(W), "val"))
            .is_some_and(|v| v != "0");
        return Some(if listed { "list" } else { "body" });
    }
    let resolved = design.formatting().paragraph(node, None).resolved;
    let listed = resolved
        .get("numPr.numId")
        .and_then(ooxml_word_model::value::Value::int)
        .is_some_and(|n| n != 0);
    Some(if listed { "list" } else { "body" })
}

/// Every paragraph style in use, per role, in role order, most used first.
pub(crate) fn styles(design: &Design<'_>, cancel: &dyn Cancel) -> Result<Vec<Value>, ToolError> {
    let names = Names::new(design.docx);
    let mut reader = Reader::new(design.docx, View::Final);
    let mut found: Vec<Usage> = Vec::new();
    for part in &design.docx.parts {
        cancel.check().map_err(crate::describe::cancelled)?;
        for entry in part.ids.paragraphs().iter() {
            let node = &entry.element;
            let declared = find_path(node, &["pPr", "pStyle"]).and_then(|s| s.attribute(Some(W), "val"));
            let style = names.applied(declared).map(|s| style_id(s).to_owned());
            let table = (part.story == "body")
                .then(|| table_above(part.root(), &entry.path))
                .flatten();
            let Some(role) = role(design, &mut reader, &names, (&part.story, node, table.is_some())) else {
                continue;
            };
            let k = match found.iter().position(|u| u.role == role && u.style == style) {
                Some(k) => k,
                None => {
                    found.push(Usage {
                        role,
                        style: style.clone(),
                        count: 0,
                        example: entry.id.clone(),
                        empty: 0,
                        tables: Tally::new(),
                    });
                    found.len() - 1
                }
            };
            let Some(usage) = found.get_mut(k) else { continue };
            usage.count += 1;
            if empty(node) {
                usage.empty += 1;
            }
            if let (Some(table), "table") = (table, role) {
                let style = find_path(&table, &["tblPr", "tblStyle"])
                    .and_then(|s| s.attribute(Some(W), "val"))
                    .unwrap_or("");
                usage.tables.add(style.to_owned());
            }
        }
    }
    let mut out = Vec::new();
    for usage in found {
        cancel.check().map_err(crate::describe::cancelled)?;
        out.push(style_entry(design, usage));
    }
    out.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.as_deref().unwrap_or("").cmp(b.2.as_deref().unwrap_or("")))
    });
    Ok(out.into_iter().map(|(_, _, _, v)| v).collect())
}

/// One style in use: its entry, with what it sorts by.
fn style_entry(design: &Design<'_>, usage: Usage) -> (usize, usize, Option<String>, Value) {
    let element = usage.style.as_deref().and_then(|s| design.docx.style(s));
    let name = element.and_then(|s| style_name(s)).map(str::to_owned);
    let table_style = usage.tables.most_common().cloned();
    let cell = table_style.as_deref().map(|t| Cell {
        style: key(t),
        conditions: Arc::from(vec![key("wholeTable")]),
    });
    let look = design.style_look(usage.style.as_deref(), cell.as_ref(), None);
    let mut entry = Map::new();
    entry.insert("role".into(), Value::from(usage.role));
    entry.insert("style".into(), name.as_deref().map_or(Value::Null, Value::from));
    entry.insert("paragraphs".into(), Value::from(usage.count));
    entry.insert("example".into(), Value::from(usage.example));
    if usage.empty > 0 {
        entry.insert("empty".into(), Value::from(usage.empty));
    }
    if let Some(table_style) = &table_style {
        let applied = design.formatting().sources().table_style(Some(table_style));
        let named = applied
            .as_deref()
            .and_then(|s| design.sheet()?.get(s))
            .map(|s| s.name.clone().unwrap_or_else(|| s.style_id.clone()));
        entry.insert("table_style".into(), named.as_deref().map_or(Value::Null, Value::from));
    }
    entry.insert("look".into(), Value::Object(design.look(&look)));
    let rank = ROLES.iter().position(|r| *r == usage.role).unwrap_or(ROLES.len());
    (rank, usage.count, name, Value::Object(entry))
}

/// The ids of the styles content applies: paragraphs', runs' and tables' in every story,
/// and the default paragraph style where a paragraph names none.
fn used_styles(docx: &Docx) -> HashSet<String> {
    let mut used = HashSet::new();
    let default = docx.default_style("paragraph").map(|s| style_id(s).to_owned());
    for part in &docx.parts {
        let mut unstyled = false;
        let mut stack = vec![part.root()];
        while let Some(e) = stack.pop() {
            for child in e.child_elements() {
                if child.name().namespace() == Some(W) {
                    match child.name().local() {
                        "pStyle" | "rStyle" | "tblStyle" => {
                            if let Some(v) = child.attribute(Some(W), "val").filter(|v| !v.is_empty()) {
                                used.insert(v.to_owned());
                            }
                        }
                        "p" => unstyled |= find_path(child, &["pPr", "pStyle"]).is_none(),
                        _ => {}
                    }
                }
                stack.push(child);
            }
        }
        if let (true, Some(default)) = (unstyled, &default) {
            used.insert(default.clone());
        }
    }
    used
}

/// `look` less what is the same in `base`: a theme colour's name kept with its colour, a
/// flag `base` has on and `look` lacks given as `false`.
pub(crate) fn differences(look: &Map<String, Value>, base: &Map<String, Value>) -> Map<String, Value> {
    let same = |a: Option<&Value>, b: Option<&Value>| match (a, b) {
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        _ => a == b,
    };
    let mut out = Map::new();
    for (key, value) in look {
        let theme_same = key
            .strip_prefix("theme_")
            .is_some_and(|plain| same(look.get(plain), base.get(plain)));
        if !same(base.get(key), Some(value)) && !theme_same {
            out.insert(key.clone(), value.clone());
        }
    }
    for name in ["color", "shading"] {
        let theme = format!("theme_{name}");
        if let (true, Some(label)) = (out.contains_key(name), look.get(&theme)) {
            out.insert(theme, label.clone());
        }
    }
    for (key, value) in base {
        if *value == Value::Bool(true) && !look.contains_key(key) {
            out.insert(key.clone(), Value::Bool(false));
        }
    }
    out
}

/// Every paragraph, character and table style the document declares, used ones first,
/// then by type, each in the document's order.
pub(crate) fn declared_styles(design: &Design<'_>, cancel: &dyn Cancel) -> Result<Vec<Value>, ToolError> {
    let docx = design.docx;
    let mut order: Vec<&str> = Vec::new();
    let mut by_id: HashMap<&str, &Arc<Element>> = HashMap::new();
    for style in docx.styles() {
        let id = style_id(style);
        if by_id.insert(id, style).is_none() {
            order.push(id);
        }
    }
    let used = used_styles(docx);
    let default = docx.default_style("paragraph").map(|s| style_id(s));
    let base = design.look(&design.style_look(default, None, None));
    let mut out: Vec<(bool, usize, Value)> = Vec::new();
    for id in order {
        cancel.check().map_err(crate::describe::cancelled)?;
        let Some(style) = by_id.get(id) else { continue };
        let kind = style_kind(style);
        let Some(rank) = ["paragraph", "character", "table"].iter().position(|k| *k == kind) else {
            continue;
        };
        let name = |s: &Element| style_name(s).unwrap_or_else(|| style_id(s)).to_owned();
        let mut entry = Map::new();
        entry.insert("name".into(), Value::from(name(style)));
        entry.insert("type".into(), Value::from(kind));
        entry.insert("used".into(), Value::Bool(used.contains(id)));
        if style.attribute(Some(W), "default").is_some_and(on) {
            entry.insert("default".into(), Value::Bool(true));
        }
        let mut chain: Vec<&str> = Vec::new();
        let mut current = child_val(style, "basedOn");
        while let Some(c) = current.filter(|c| by_id.contains_key(c) && !chain.contains(c) && *c != id) {
            chain.push(c);
            current = by_id.get(c).and_then(|s| child_val(s, "basedOn"));
        }
        if !chain.is_empty() {
            let names: Vec<Value> = chain
                .iter()
                .filter_map(|c| by_id.get(c))
                .map(|s| Value::from(name(s)))
                .collect();
            entry.insert("based_on".into(), Value::Array(names));
        }
        match kind {
            "paragraph" => {
                let look = design.look(&design.style_look(Some(id), None, None));
                let whole = default == Some(id);
                entry.insert(
                    "look".into(),
                    Value::Object(if whole { look } else { differences(&look, &base) }),
                );
            }
            "character" => {
                let look = design.style_look(None, None, Some(id));
                let text = design.text_look(&look.run, &look.run_origins);
                entry.insert("look".into(), Value::Object(differences(&text, &base)));
            }
            _ => table_entry(design, style, &chain, &base, &mut entry),
        }
        out.push((!used.contains(id), rank, Value::Object(entry)));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    Ok(out.into_iter().map(|(_, _, v)| v).collect())
}

/// A declared table style's regions (`parts`) and its places under the options its regions
/// turn on: each one's fill, borders drawn (sides with the same border joined) and text
/// where it differs from the default paragraph style's.
fn table_entry(
    design: &Design<'_>,
    style: &Element,
    chain: &[&str],
    base: &Map<String, Value>,
    entry: &mut Map<String, Value>,
) {
    let id = style_id(style);
    let mut parts: Vec<String> = Vec::new();
    for s in std::iter::once(id).chain(chain.iter().copied()) {
        let Some(node) = design.docx.style(s) else { continue };
        for region in node.child_elements().filter(|c| c.name().is(Some(W), "tblStylePr")) {
            if let Some(kind) = region.attribute(Some(W), "type").filter(|k| !k.is_empty()) {
                if !parts.iter().any(|p| p == kind) {
                    parts.push(kind.to_owned());
                }
            }
        }
    }
    let has = |kind: &str| parts.iter().any(|p| p == kind);
    let [first_row, last_row, first_column, last_column] = PART_OPTIONS.map(has);
    let look = Look {
        first_row,
        last_row,
        first_column,
        last_column,
        no_h_band: !(has("band1Horz") || has("band2Horz")),
        no_v_band: true,
    };
    entry.insert("parts".into(), Value::from(parts.clone()));
    let mut compact_places = Map::new();
    for (name, place) in places(design, id, look, None) {
        let mut compact = Map::new();
        for key in ["fill", "theme_fill"] {
            if let Some(value) = place.get(key).filter(|v| truthy(v)) {
                compact.insert(key.into(), value.clone());
            }
        }
        let mut drawn: Vec<(String, Vec<String>)> = Vec::new();
        if let Some(Value::Object(sides)) = place.get("borders") {
            for (side, border) in sides {
                let border = border.as_str().unwrap_or("none");
                if border == "none" {
                    continue;
                }
                match drawn.iter_mut().find(|(b, _)| b == border) {
                    Some((_, list)) => list.push(side.clone()),
                    None => drawn.push((border.to_owned(), vec![side.clone()])),
                }
            }
        }
        if !drawn.is_empty() {
            let borders: Map<String, Value> = drawn
                .into_iter()
                .map(|(border, sides)| (sides.join(","), Value::from(border)))
                .collect();
            compact.insert("borders".into(), Value::Object(borders));
        }
        if let Some(Value::Object(text)) = place.get("text") {
            let text = differences(text, base);
            if !text.is_empty() {
                compact.insert("text".into(), Value::Object(text));
            }
        }
        compact_places.insert(name, Value::Object(compact));
    }
    entry.insert("places".into(), Value::Object(compact_places));
}

/// Python's truth of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}
