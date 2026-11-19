//! Owns: what `word_styles` reports of a style and of the document's languages, as
//! docx-agent's `describe_style`, `looks_off`, `_look_warnings` and `_set_language` write
//! it (`tools/style.py`): a style's declared formatting, tab stops, regions (and the
//! tables whose look hides one), the list level it numbers at, whether new tables get it,
//! and where it is used; the languages the document states.
//!
//! Must not: change anything.

use crate::objects::{read_look, LOOK_NAMES};
use ooxml_tools::python_dumps;
use ooxml_word_edit::{
    declared, looks_needed, read_regions, read_shading, read_tabs, style_link, table_default, usage, Declared,
    EditError, Languages, Region, StyleLink,
};
use ooxml_word_model::docx::{style_id, style_kind, style_name, Docx};
use ooxml_word_model::ids::W;
use ooxml_xml::Element;
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// A declared value as JSON (a line rule as `[rule, points]`).
fn value(declared: &Declared) -> Value {
    match declared {
        Declared::Flag(on) => json!(on),
        Declared::Number(n) => json!(n),
        Declared::Text(t) => json!(t),
        Declared::Rule(rule, points) => json!([rule, points]),
    }
}

/// A region as `read_regions` reports it.
fn region(name: &str, region: &Region) -> Value {
    let mut out = Map::new();
    let fields = [
        ("color", region.color.clone().map(Value::from)),
        ("bold", region.bold.map(Value::from)),
        ("italic", region.italic.map(Value::from)),
        ("size", region.size.map(|s| json!(s))),
        ("alignment", region.alignment.clone().map(Value::from)),
        ("shading", region.shading.clone().map(Value::from)),
    ];
    for (k, v) in fields {
        if let Some(v) = v {
            out.insert(k.to_owned(), v);
        }
    }
    if !region.borders.is_empty() {
        let borders: Map<String, Value> = region
            .borders
            .iter()
            .map(|(side, words)| ((*side).to_owned(), json!(words)))
            .collect();
        out.insert("borders".to_owned(), Value::Object(borders));
    }
    if name == "whole_table_part" {
        out.insert(
            "note".to_owned(),
            json!("w:tblStylePr wholeTable: Word draws it nowhere"),
        );
    }
    if let Some(size) = region.band_size {
        out.insert("band_size".to_owned(), json!(size));
    }
    Value::Object(out)
}

/// A table style's regions as JSON (`{region: {field: value}}`).
#[must_use]
pub(crate) fn regions_json(style: &Element) -> Map<String, Value> {
    read_regions(style)
        .iter()
        .map(|(name, r)| (name.clone(), region(name, r)))
        .collect()
}

/// The `w:style` docx-agent's `_style_element` finds: the first with the id.
fn element<'d>(docx: &'d Docx, id: &str) -> Option<&'d Arc<Element>> {
    docx.styles().iter().find(|s| style_id(s) == id)
}

/// The tables in table style `id` whose look turns off a region the style formats
/// (`looks_off`): each with the options off and the `word_format_table` call showing them.
#[must_use]
pub(crate) fn looks_off(docx: &Docx, id: &str) -> Vec<Value> {
    let regions = element(docx, id).map(|s| read_regions(s)).unwrap_or_default();
    let wanted = looks_needed(
        regions
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| ooxml_word_edit::LOOK_OF.iter().any(|(r, _)| r == n)),
    );
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for story in &docx.parts {
        for table in story.ids.tables().iter() {
            let properties = table.element.find(Some(W), "tblPr");
            let named = properties
                .and_then(|p| p.find(Some(W), "tblStyle"))
                .and_then(|s| s.attribute(Some(W), "val"));
            if named != Some(id) {
                continue;
            }
            let look = read_look(properties.and_then(|p| p.find(Some(W), "tblLook")).map(|l| &**l));
            let on: Vec<&str> = LOOK_NAMES
                .iter()
                .zip(look)
                .filter(|(_, on)| *on)
                .map(|(n, _)| *n)
                .collect();
            let off: Vec<&str> = wanted.iter().copied().filter(|n| !on.contains(n)).collect();
            if off.is_empty() {
                continue;
            }
            let mut fix: Vec<&str> = on.clone();
            fix.extend(off.iter().copied().filter(|n| !on.contains(n)));
            out.push(json!({
                "table": table.id,
                "off": off,
                "fix": {"tool": "word_format_table", "table": table.id, "scope": "table", "look": fix},
            }));
        }
    }
    out
}

/// After a table style's regions change: the tables in it that will not show them
/// (`_look_warnings`).
#[must_use]
pub(crate) fn look_warnings(docx: &Docx, name: &str) -> Vec<String> {
    let Some(style) = docx.find_style(name, "table") else {
        return Vec::new();
    };
    let off = looks_off(docx, style_id(style));
    let Some(first) = off.first() else {
        return Vec::new();
    };
    let named: Vec<String> = off
        .iter()
        .take(5)
        .map(|e| {
            let offs: Vec<&str> = e["off"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            format!("{} has {} off", e["table"].as_str().unwrap_or(""), offs.join(", "))
        })
        .collect();
    let more = if off.len() > 5 {
        format!(" and {} more", off.len() - 5)
    } else {
        String::new()
    };
    // the fix's call less its tool, its keys in order
    let fix: serde_json::Map<String, Value> = first["fix"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| *k != "tool")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let fix = Value::Object(fix);
    vec![format!(
        "{} table(s) in {} do not show a region it formats ({}{more}): a region shows where the table's look turns it on -- word_format_table {}",
        off.len(),
        ooxml_base::pyfmt::repr_str(name),
        named.join("; "),
        python_dumps(&fix)
    )]
}

/// A style's description (`describe_style`).
///
/// # Errors
/// When a part is not XML.
pub(crate) fn describe(docx: &Docx, style: &Arc<Element>) -> Result<Value, EditError> {
    let id = style_id(style);
    let kind = style_kind(style);
    let name = style_name(style);
    let based_on = style.find(Some(W), "basedOn").and_then(|b| b.attribute(Some(W), "val"));
    let mut data = Map::new();
    data.insert("name".to_owned(), json!(name));
    data.insert("kind".to_owned(), json!(kind));
    data.insert("based_on".to_owned(), json!(based_on));
    if let Some(node) = element(docx, id) {
        let paragraph = kind == "paragraph";
        let mut found: Map<String, Value> = declared(node, paragraph)
            .iter()
            .map(|(n, v)| ((*n).to_owned(), value(v)))
            .collect();
        let holder = node.find(Some(W), if paragraph { "pPr" } else { "rPr" });
        if let Some(shading) = read_shading(holder.map(|h| &**h)) {
            found.insert("shading".to_owned(), json!(shading));
        }
        if let Some(tabs) = read_tabs(node).filter(|_| paragraph) {
            let stops: Vec<Value> = tabs
                .iter()
                .map(|t| {
                    let mut stop = json!({"position": t.position, "alignment": t.alignment});
                    if let (Some(leader), Some(map)) = (&t.leader, stop.as_object_mut()) {
                        map.insert("leader".to_owned(), json!(leader));
                    }
                    stop
                })
                .collect();
            found.insert("tabs".to_owned(), Value::Array(stops));
        }
        data.insert("declared".to_owned(), Value::Object(found));
        if kind == "table" {
            describe_regions(docx, node, &mut data);
        }
        if paragraph {
            if let Some(link) = style_link(docx, id) {
                data.insert("numbering".to_owned(), link_json(docx, &link));
            }
        }
    }
    if kind == "table" && table_default(docx).is_some_and(|d| style_id(&d) == id) {
        data.insert("default".to_owned(), json!(true));
    }
    // docx-agent counts the uses of the style its name finds
    let counted = name.and_then(|n| docx.find_style(n, kind)).map_or(id, |s| style_id(s));
    let used = usage(docx, counted)?;
    data.insert(
        "usage".to_owned(),
        json!({"content": used.content, "styles": used.styles}),
    );
    Ok(Value::Object(data))
}

fn describe_regions(docx: &Docx, node: &Element, data: &mut Map<String, Value>) {
    let regions = regions_json(node);
    if regions.is_empty() {
        return;
    }
    // in the style's order (the map may sort its keys)
    let order: Vec<String> = read_regions(node).into_iter().map(|(n, _)| n).collect();
    let looks = looks_needed(order.iter().map(String::as_str));
    data.insert("regions".to_owned(), Value::Object(regions));
    data.insert("shown_with_look".to_owned(), json!(looks));
    let off = looks_off(docx, style_id(node));
    if !off.is_empty() {
        data.insert(
            "tables_with_look_off".to_owned(),
            json!(off.iter().take(10).collect::<Vec<_>>()),
        );
        if off.len() > 10 {
            data.insert("tables_with_look_off_left_out".to_owned(), json!(off.len() - 10));
        }
    }
}

fn link_json(docx: &Docx, link: &StyleLink) -> Value {
    match link {
        StyleLink::None => json!({"list": "none"}),
        StyleLink::Level { list, level, own, from } => {
            let mut out = json!({"list": list, "level": level, "own": own});
            if !own {
                let named = from.as_deref().map(|f| {
                    docx.styles()
                        .iter()
                        .find(|s| style_id(s) == f)
                        .map_or(f, |s| style_name(s).filter(|n| !n.is_empty()).unwrap_or(f))
                        .to_owned()
                });
                if let Some(map) = out.as_object_mut() {
                    map.insert("from".to_owned(), json!(named));
                }
            }
            out
        }
    }
}

fn tags(found: &[(&'static str, String)]) -> Value {
    Value::Object(found.iter().map(|(k, v)| ((*k).to_owned(), json!(v))).collect())
}

/// `set_language`'s data: the default, the first 20 styles with their own, the runs'.
#[must_use]
pub(crate) fn languages_json(found: &Languages) -> Value {
    let mut data = Map::new();
    data.insert("default".to_owned(), tags(&found.default));
    let styles: Map<String, Value> = found
        .styles
        .iter()
        .take(20)
        .map(|(n, t)| (n.clone(), tags(t)))
        .collect();
    data.insert("styles".to_owned(), Value::Object(styles));
    if found.styles.len() > 20 {
        data.insert("styles_left_out".to_owned(), json!(found.styles.len() - 20));
    }
    if !found.runs.is_empty() {
        let runs: Map<String, Value> = found.runs.iter().map(|(t, n)| (t.clone(), json!(n))).collect();
        data.insert("runs".to_owned(), Value::Object(runs));
    }
    Value::Object(data)
}
