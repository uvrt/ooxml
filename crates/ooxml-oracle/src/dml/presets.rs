//! Owns: `drawingml-model`'s two table checks -- every preset geometry with its guides
//! evaluated at one size, precise and rounded (`--presets`), and colour swatches read from
//! standard input and resolved under both applications' rules (`--swatches`) -- in the form
//! `python/drawingml_diff.py` writes ooxml-common's answers in.
//!
//! Must not: be used by any library; it is a test instrument.

use super::model::{num, spec_path};
use super::resolved::colour;
use ooxml_colour::{apply_linear, apply_transforms, Transform, TransformKind, POWERPOINT, WORD};
use ooxml_drawingml::guides::evaluate_guides;
use ooxml_drawingml::presets::presets;
use serde_json::{json, Map, Value as J};

/// Every preset, with its guides evaluated in a 120 x 72 box.
pub fn table() -> J {
    let rows: Vec<J> = presets()
        .iter()
        .map(|p| {
            let adjustments: Vec<(String, String)> = p
                .adjustments
                .iter()
                .map(|(n, d)| (n.clone(), format!("val {d}")))
                .collect();
            let evaluate = |precise: bool| {
                let values = evaluate_guides(&[&adjustments, &p.guides], 120.0, 72.0, precise);
                let mut sorted: Vec<(&String, &f64)> = values.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(b.0));
                J::Object(
                    sorted
                        .into_iter()
                        .map(|(k, v)| (k.clone(), num(*v)))
                        .collect::<Map<_, _>>(),
                )
            };
            json!({
                "name": p.name,
                "adjustments": p.adjustments.iter().map(|(n, d)| json!([n, num(*d)])).collect::<Vec<_>>(),
                "guides": p.guides.iter().map(|(n, f)| json!([n, f])).collect::<Vec<_>>(),
                "paths": p.paths.iter().map(spec_path).collect::<Vec<_>>(),
                "text_rect": p.text_rect,
                "precise": evaluate(true),
                "rounded": evaluate(false),
            })
        })
        .collect();
    J::Array(rows)
}

fn swatch(case: &J) -> J {
    let transforms: Vec<Transform> = case["transforms"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            Some(Transform {
                kind: TransformKind::from_name(t.get(0)?.as_str()?)?,
                value: t.get(1)?.as_f64()?,
            })
        })
        .collect();
    let linear = case["linear"].as_array().and_then(|l| {
        let v: Vec<f64> = l.iter().filter_map(J::as_f64).collect();
        <[f64; 3]>::try_from(v).ok()
    });
    let hex = case["hex"].as_str().unwrap_or("000000");
    let mut out = Map::new();
    for rules in [POWERPOINT, WORD] {
        let resolved = match linear {
            Some(l) => Ok(apply_linear(l, &transforms, &rules)),
            None => apply_transforms(hex, &transforms, &rules),
        };
        let value = resolved.map_or_else(|_| json!({"error": "colour"}), |c| colour(&c));
        out.insert(rules.name.to_owned(), value);
    }
    J::Object(out)
}

/// Each line of `input` (a swatch: `hex`, `linear`, `transforms`) resolved, one JSON line each.
pub fn swatches(input: &str) -> Vec<String> {
    input
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<J>(line)
                .map(|case| swatch(&case).to_string())
                .unwrap_or_else(|e| json!({"error": e.to_string()}).to_string())
        })
        .collect()
}
