//! Owns: every ECMA-376 preset geometry as data -- its adjustments with their defaults, its
//! guides in evaluation order, its paths as written and its text rectangle -- read once
//! from `data/presets.tsv`, which `data/generate.py` writes from ooxml-common's tables (one
//! source: the specification's `presetShapeDefinitions.xml`, compiled and pinned there).
//!
//! Must not: evaluate or draw a preset (the guides evaluate with [`crate::guides`]; drawing
//! is phase P3c's), or hold a preset written by hand.

use crate::source::{SpecCommand, SpecPath};
use std::collections::HashMap;
use std::sync::OnceLock;

/// One preset geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    /// The preset's name (`ST_ShapeType`).
    pub name: String,
    /// `a:avLst`: adjustments and their defaults, in 1/1000 percent or 1/60000 degree.
    pub adjustments: Vec<(String, f64)>,
    /// `a:gdLst`: guides and their formulas, in evaluation order.
    pub guides: Vec<(String, String)>,
    /// `a:pathLst`: the paths as written.
    pub paths: Vec<SpecPath>,
    /// `a:rect`: `l`, `t`, `r`, `b` as guide names or literals.
    pub text_rect: [String; 4],
}

struct Table {
    presets: Vec<Preset>,
    by_name: HashMap<String, usize>,
}

fn start(fields: &[&str]) -> Option<Preset> {
    let field = |i: usize| fields.get(i).map(|f| (*f).to_owned());
    Some(Preset {
        name: field(1)?,
        adjustments: Vec::new(),
        guides: Vec::new(),
        paths: Vec::new(),
        text_rect: [field(2)?, field(3)?, field(4)?, field(5)?],
    })
}

fn path(fields: &[&str]) -> Option<SpecPath> {
    let space = match fields.get(3)? {
        &"-" => None,
        written => {
            let (w, h) = written.split_once(' ')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        }
    };
    Some(SpecPath {
        fill: (*fields.get(1)?).to_owned(),
        stroke: *fields.get(2)? == "1",
        space,
        commands: Vec::new(),
    })
}

/// One record into the preset being read; `None` where the record is malformed.
fn record(presets: &mut Vec<Preset>, fields: &[&str]) -> Option<()> {
    if fields.first() == Some(&"preset") {
        presets.push(start(fields)?);
        return Some(());
    }
    let preset = presets.last_mut()?;
    let field = |i: usize| fields.get(i).map(|f| (*f).to_owned());
    match *fields.first()? {
        "adj" => preset.adjustments.push((field(1)?, field(2)?.parse().ok()?)),
        "gd" => preset.guides.push((field(1)?, field(2)?)),
        "path" => preset.paths.push(path(fields)?),
        "cmd" => preset.paths.last_mut()?.commands.push(SpecCommand {
            letter: field(1)?.chars().next()?,
            operands: fields.iter().skip(2).map(|f| (*f).to_owned()).collect(),
        }),
        _ => return None,
    }
    Some(())
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut presets = Vec::new();
        for line in include_str!("../data/presets.tsv").lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            // The table is generated and checked; a malformed record would drop only itself,
            // and the crate's tests count every preset.
            let _ = record(&mut presets, &fields);
        }
        let by_name = presets.iter().enumerate().map(|(i, p)| (p.name.clone(), i)).collect();
        Table { presets, by_name }
    })
}

/// Every preset, in the table's order.
pub fn presets() -> &'static [Preset] {
    &table().presets
}

/// The preset named `name` (`ST_ShapeType`, case as written).
pub fn preset(name: &str) -> Option<&'static Preset> {
    let table = table();
    table.by_name.get(name).and_then(|i| table.presets.get(*i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_the_schema_allows_is_there() {
        assert_eq!(presets().len(), 187);
        let rect = preset("roundRect").unwrap();
        assert_eq!(rect.adjustments, [("adj".to_owned(), 16667.0)]);
        assert!(!rect.paths.is_empty() && rect.paths.iter().all(|p| !p.commands.is_empty()));
        assert!(preset("noSuchPreset").is_none());
    }
}
