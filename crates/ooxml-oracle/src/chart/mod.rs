//! Owns: the chart differential's Rust side -- every chart part of a package (or a bare
//! chart XML file) read by `ooxml-chart`, written as JSON in the form
//! `python/chart_diff.py` writes ooxml-common's for the same files ([`model`]); and each
//! chart laid out in a frame under an application's rules ([`laid`]).
//!
//! Must not: be used by any library; it is a test instrument.

pub mod laid;
pub mod model;

use ooxml_base::Limits;
use ooxml_opc::Package;
use serde_json::{json, Map, Value as J};
use std::sync::Arc;

/// One chart part's record: its model, or why it has none.
fn part(root: &ooxml_xml::Element) -> J {
    match ooxml_chart::chart_space(root) {
        Some(chart) => json!({"model": model::chart(&chart)}),
        None => json!({"model": null}),
    }
}

/// The record of one file: its chart parts (`c:chartSpace` roots) in name order, or a bare
/// XML file as one part.
///
/// # Errors
/// When the file cannot be read or opened.
pub fn file(path: &str) -> Result<J, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let mut parts = Map::new();
    if path.ends_with(".xml") {
        let tree = ooxml_xml::parse(&bytes, &Limits::default().xml).map_err(|e| e.to_string())?;
        parts.insert(path.to_owned(), part(tree.root()));
        return Ok(json!({"file": path, "parts": parts}));
    }
    let package = Package::open(Arc::from(bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let mut names: Vec<String> = package.part_names();
    names.sort();
    for name in names.iter().filter(|n| n.ends_with(".xml")) {
        if let Ok(Some(tree)) = package.tree(name) {
            if tree.root().name().local() == "chartSpace" {
                parts.insert(name.clone(), part(tree.root()));
            }
        }
    }
    Ok(json!({"file": path, "parts": parts}))
}

/// The chart part a layout case names: a bare XML file's root, or a package's part.
fn chart_of(spec: &J) -> Result<Option<ooxml_chart::model::Chart>, String> {
    let path = spec.get("file").and_then(J::as_str).ok_or("a case names its file")?;
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if path.ends_with(".xml") {
        let tree = ooxml_xml::parse(&bytes, &Limits::default().xml).map_err(|e| e.to_string())?;
        return Ok(ooxml_chart::chart_space(tree.root()));
    }
    let part = spec
        .get("part")
        .and_then(J::as_str)
        .ok_or("a package case names its part")?;
    let package = Package::open(Arc::from(bytes), &Limits::default()).map_err(|e| e.to_string())?;
    match package.tree(part) {
        Ok(Some(tree)) => Ok(ooxml_chart::chart_space(tree.root())),
        _ => Err(format!("{path}: no part {part}")),
    }
}

/// The locale a case states (en-US where it states none).
fn locale_of(spec: &J) -> Result<ooxml_numfmt::Locale, String> {
    let Some(settings) = spec.get("locale").filter(|l| !l.is_null()) else {
        return Ok(ooxml_numfmt::EN_US);
    };
    let text = |key: &str| settings.get(key).and_then(J::as_str).unwrap_or("");
    let one = |key: &str, default: char| text(key).chars().next().unwrap_or(default);
    ooxml_numfmt::Locale::new(
        one("decimal_separator", '.'),
        one("thousands_separator", ','),
        text("date_order"),
        text("language"),
        one("list_separator", ','),
    )
    .map_err(|e| e.to_string())
}

/// One layout case (a JSON object: `file`, `part`, `width_pt`, `height_pt`, `rules`,
/// `locale`) laid out.
///
/// # Errors
/// When the case cannot be read.
pub fn layout(spec: &J) -> Result<J, String> {
    let Some(chart) = chart_of(spec)? else {
        return Ok(json!({"notes": ["no c:chart"]}));
    };
    let number = |key: &str| spec.get(key).and_then(J::as_f64).ok_or(format!("a case states {key}"));
    let case = laid::Case {
        chart: &chart,
        width: number("width_pt")?,
        height: number("height_pt")?,
        rules: spec.get("rules").and_then(J::as_str).unwrap_or("powerpoint"),
        locale: locale_of(spec)?,
        variant: spec.get("variant").and_then(J::as_str),
    };
    Ok(laid::case(&case))
}
