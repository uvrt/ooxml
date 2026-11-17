//! Owns: the chart differential's Rust side -- every chart part of a package (or a bare
//! chart XML file) read by `ooxml-chart`, written as JSON in the form
//! `python/chart_diff.py` writes ooxml-common's for the same files ([`model`]).
//!
//! Must not: be used by any library; it is a test instrument.

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
