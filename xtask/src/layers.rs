//! `cargo xtask check-layers`: the crate graph against the layer map (design sections 2.1
//! and 3).
//!
//! Owns: the layer map -- each crate's layer, and its rank inside the layer where the
//! design orders crates within one (`ooxml-base` under the other layer-0 crates; the scene
//! under what lowers to it; the tool contract's derive under the contract, and the contract
//! under each format's tools) --
//! and the check: every dependency of a crate, read from `cargo metadata`, points to a
//! lower place in the map. A sideways or upward edge fails, as does a crate the map does
//! not place; the allow-list of exceptions ([`ALLOWED`]) starts empty.
//!
//! Must not: change the graph or the map to make an edge pass. The prototype crates
//! (`crates/core`, `crates/word`, `crates/python`) are outside the map until they are
//! replaced: they may depend on the new crates, never the other way round. Test support
//! (`ooxml-oracle`) may depend on any crate and is depended on by tests only.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// Where a crate stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// In layer `.0` at rank `.1` (lower first): it may depend only on a lower place.
    Layer(u8, u8),
    /// Test support: may depend on anything; only tests may depend on it.
    TestSupport,
    /// A prototype crate awaiting its replacement: may depend on anything; nothing in the
    /// map may depend on it.
    Prototype,
}

/// The layer map: every crate of the workspace.
pub const MAP: &[(&str, Place)] = &[
    ("ooxml-base", Place::Layer(0, 0)),
    ("ooxml-xml", Place::Layer(0, 1)),
    ("ooxml-zip", Place::Layer(0, 1)),
    ("ooxml-store", Place::Layer(0, 1)),
    ("ooxml-derive", Place::Layer(0, 1)),
    ("ooxml-opc", Place::Layer(1, 0)),
    ("ooxml-fonts", Place::Layer(1, 0)),
    ("ooxml-text", Place::Layer(1, 0)),
    ("ooxml-schema", Place::Layer(1, 0)),
    ("ooxml-colour", Place::Layer(1, 0)),
    ("ooxml-numfmt", Place::Layer(1, 0)),
    ("ooxml-style-names", Place::Layer(1, 0)),
    ("ooxml-markdown", Place::Layer(1, 0)),
    ("ooxml-drawingml", Place::Layer(2, 0)),
    // Charts and metafiles lower to a scene (design section 8): the scene under them; charts
    // lower to DrawingML's drawable elements too, so they rank above it.
    ("ooxml-scene", Place::Layer(2, 0)),
    ("ooxml-metafile", Place::Layer(2, 1)),
    ("ooxml-chart", Place::Layer(2, 1)),
    // The DrawingML painter: drawables (a chart's, a slide's, a document's) onto the scene.
    ("ooxml-render", Place::Layer(2, 1)),
    // The scene's PNG adapter: the typed writer's SVG drawn by resvg (decision 11).
    ("ooxml-raster", Place::Layer(2, 1)),
    // ooxml-word in the design; the prototype holds that package name (lane L2)
    ("ooxml-word-model", Place::Layer(3, 0)),
    ("ooxml-word-layout", Place::Layer(3, 1)),
    ("ooxml-word-pages", Place::Layer(3, 2)),
    // Word's edits: commands over the model, through the store (phase P2)
    ("ooxml-word-edit", Place::Layer(3, 1)),
    // Workbooks read (a chart's embedded workbook first; the sheets phase builds on it)
    ("ooxml-sheets", Place::Layer(3, 0)),
    ("ooxml-tools-derive", Place::Layer(4, 0)),
    ("ooxml-tools", Place::Layer(4, 1)),
    ("ooxml-tools-word", Place::Layer(4, 2)),
    ("ooxml-session", Place::Layer(5, 0)),
    // ooxml-python in the design; the prototype holds that package name (lane P0d)
    ("ooxml-python-session", Place::Layer(6, 0)),
    ("xtask", Place::Layer(6, 0)),
    ("ooxml-oracle", Place::TestSupport),
    ("ooxml-core", Place::Prototype),
    ("ooxml-word", Place::Prototype),
    ("ooxml-python", Place::Prototype),
];

/// Edges allowed against the map, `(from, to)`, each with its reason. Empty, and to stay
/// so: an exception needs an ADR.
pub const ALLOWED: &[(&str, &str)] = &[];

/// A dependency between two workspace crates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// The crate that depends.
    pub from: String,
    /// The crate it depends on.
    pub to: String,
    /// Whether only its tests, examples and benchmarks do (`[dev-dependencies]`).
    pub dev: bool,
}

/// The workspace's crates and their dependencies on each other, from `cargo metadata`.
///
/// # Errors
/// When cargo cannot be run or its output is not what `--format-version 1` gives.
pub fn edges(root: &Path) -> Result<(Vec<String>, Vec<Edge>), String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(root)
        .args(["metadata", "--format-version", "1", "--no-deps", "--offline"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!("cargo metadata: {}", String::from_utf8_lossy(&output.stderr)));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("cargo metadata: no packages")?;
    let name = |p: &Value| p.get("name").and_then(Value::as_str).map(str::to_owned);
    let names: Vec<String> = packages.iter().filter_map(name).collect();
    let mut found = Vec::new();
    for package in packages {
        let Some(from) = name(package) else { continue };
        let deps = package.get("dependencies").and_then(Value::as_array);
        for dep in deps.into_iter().flatten() {
            let Some(to) = dep.get("name").and_then(Value::as_str) else {
                continue;
            };
            if names.iter().any(|n| n == to) {
                let dev = dep.get("kind").and_then(Value::as_str) == Some("dev");
                found.push(Edge {
                    from: from.clone(),
                    to: to.to_owned(),
                    dev,
                });
            }
        }
    }
    Ok((names, found))
}

/// Every way `crates` and `edges` break the map: crates it does not place, and edges that
/// go sideways, upward, into a prototype, or (outside tests) into test support.
#[must_use]
pub fn problems(crates: &[String], edges: &[Edge], map: &[(&str, Place)], allowed: &[(&str, &str)]) -> Vec<String> {
    let places: BTreeMap<&str, Place> = map.iter().copied().collect();
    let mut out: Vec<String> = crates
        .iter()
        .filter(|c| !places.contains_key(c.as_str()))
        .map(|c| format!("{c} is not in the layer map (xtask/src/layers.rs): give it its layer"))
        .collect();
    for edge in edges {
        if allowed.contains(&(edge.from.as_str(), edge.to.as_str())) {
            continue;
        }
        let (Some(from), Some(to)) = (places.get(edge.from.as_str()), places.get(edge.to.as_str())) else {
            continue;
        };
        let wrong = match (*from, *to) {
            (Place::Prototype | Place::TestSupport, _) => None,
            (_, Place::Prototype) => Some("into a prototype crate"),
            (_, Place::TestSupport) if !edge.dev => Some("into test support outside tests"),
            (_, Place::TestSupport) => None,
            (Place::Layer(a, i), Place::Layer(b, j)) if (b, j) == (a, i) => Some("sideways"),
            (Place::Layer(a, i), Place::Layer(b, j)) if (b, j) > (a, i) => Some("upward"),
            (Place::Layer(..), Place::Layer(..)) => None,
        };
        if let Some(wrong) = wrong {
            let kind = if edge.dev { " (dev)" } else { "" };
            out.push(format!(
                "{} -> {}{kind}: {wrong} ({:?} -> {:?})",
                edge.from, edge.to, from, to
            ));
        }
    }
    out
}

/// Check the workspace at `root`: the problems, empty when the layers hold.
///
/// # Errors
/// When cargo's metadata cannot be read.
pub fn run(root: &Path) -> Result<Vec<String>, String> {
    let (crates, edges) = edges(root)?;
    Ok(problems(&crates, &edges, MAP, ALLOWED))
}
