//! Owns: a run's (and a paragraph mark's) properties through the cascade, with Word's
//! toggle rule, the legacy 12 pt table-cell size (measured by
//! `make_table_style_size_probe.py`) and a table of contents' link drawn with little of
//! its `Hyperlink` style (measured, round 31) -- docx2svg's `_resolve_run`.
//!
//! Must not: choose a face per character (`crate::resolve::fonts`).

use super::cascade::{flatten, merge, Part, Sources, LEGACY_DEFAULT_SIZE, LEGACY_UNSET_DEFAULT};
use crate::props::is_toggle;
use crate::styles::StyleKind;
use crate::value::{key, Props, Text, Value};
use std::collections::BTreeSet;

/// What a run's resolution reads of the run and its paragraph.
#[derive(Debug, Clone, Copy)]
pub struct RunInput<'a> {
    /// The paragraph's `w:pStyle`.
    pub paragraph_style: Option<&'a str>,
    /// The innermost table's style (`""`: none named), `None` outside tables.
    pub table_style: Option<&'a str>,
    /// The cell's regions.
    pub conditions: &'a [Text],
    /// `w:rStyle` (`None` for a paragraph mark).
    pub character_style: Option<&'a str>,
    /// A table of contents' link.
    pub toc_link: bool,
    /// Direct formatting.
    pub direct: &'a Props,
}

/// The `w:name`s (without case) of the styles a table of contents' link draws with little
/// of their formatting.
const TOC_LINK_STYLES: &[&str] = &["hyperlink", "followedhyperlink"];

/// What of a `Hyperlink` character style a table of contents' entry takes: its fonts and
/// its capitals (`w:smallCaps` as `w:caps`).
fn toc_link_layer(level: &Props) -> Props {
    let mut out: Props = level
        .iter()
        .filter(|(k, _)| k.starts_with("rFonts."))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let capitals: Vec<&Value> = ["caps", "smallCaps"].iter().filter_map(|k| level.get(*k)).collect();
    if !capitals.is_empty() {
        let on = capitals.iter().any(|v| v.truthy());
        out.insert(key("caps"), Value::Bool(on));
    }
    out
}

/// Whether a cell's text in a 12 pt paragraph style takes the table style's size instead.
fn legacy_table_size(src: &Sources<'_>, table_id: &str) -> bool {
    if src.settings.compatibility_mode.is_some_and(|m| m >= 15) {
        return false;
    }
    if matches!(
        src.settings
            .compat_setting("overrideTableStyleFontSizeAndJustification")
            .unwrap_or("0"),
        "1" | "true" | "on"
    ) {
        return false;
    }
    src.styles
        .map(|s| s.chain(Some(table_id), StyleKind::Table))
        .unwrap_or_default()
        .iter()
        .any(|s| !s.paragraph.is_empty() || !s.run.is_empty())
}

fn non_toggles(props: &Props) -> Props {
    props
        .iter()
        .filter(|(k, _)| !is_toggle(k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn style_levels(src: &Sources<'_>, input: &RunInput<'_>, defaults: &Props) -> Vec<Props> {
    let mut levels = Vec::new();
    let table = input.table_style.and_then(|t| src.table_style(Some(t)));
    if let Some(id) = &table {
        let chain = src
            .styles
            .map(|s| s.chain(Some(id), StyleKind::Table))
            .unwrap_or_default();
        let named = super::cascade::banded(&chain, input.conditions);
        levels.push(flatten(&chain, Part::Run, &named));
    }
    let style_id = src.paragraph_style(input.paragraph_style);
    let chain = src
        .styles
        .map(|s| s.chain(style_id.as_deref(), StyleKind::Paragraph))
        .unwrap_or_default();
    let mut paragraph_level = flatten(&chain, Part::Run, &[]);
    if table.as_deref().is_some_and(|id| legacy_table_size(src, id)) {
        let table_level = levels.first().cloned().unwrap_or_default();
        paragraph_level.retain(|k, v| {
            let gives_way = matches!(&**k, "sz" | "szCs")
                && v.int() == Some(LEGACY_DEFAULT_SIZE)
                && matches!(v, Value::Int(_))
                && (table_level.contains_key(k)
                    || defaults.get(k).is_some_and(|d| d.int() != Some(LEGACY_UNSET_DEFAULT)));
            !gives_way
        });
    }
    levels.push(paragraph_level);
    if let Some(character) = input.character_style.filter(|c| !c.is_empty()) {
        let chain = src
            .styles
            .map(|s| s.chain(Some(character), StyleKind::Character))
            .unwrap_or_default();
        let mut level = flatten(&chain, Part::Run, &[]);
        let named = chain
            .last()
            .and_then(|s| s.name.as_deref())
            .map(str::to_lowercase)
            .unwrap_or_default();
        if input.toc_link && TOC_LINK_STYLES.contains(&named.as_str()) {
            level = toc_link_layer(&level);
        }
        levels.push(level);
    }
    levels
}

/// A run's (or a mark's) properties through the whole hierarchy.
pub fn resolve_run(src: &Sources<'_>, input: &RunInput<'_>) -> Props {
    let empty = Props::new();
    let defaults = src.styles.map_or(&empty, |s| &s.run_defaults);
    let levels = style_levels(src, input, defaults);
    let mut layer = non_toggles(defaults);
    for level in &levels {
        merge(&mut layer, &non_toggles(level));
    }
    merge(&mut layer, &non_toggles(input.direct));
    let declared: BTreeSet<&Text> = defaults
        .keys()
        .chain(input.direct.keys())
        .chain(levels.iter().flat_map(|l| l.keys()))
        .filter(|k| is_toggle(k))
        .collect();
    for k in declared {
        let base = defaults.get(k).is_some_and(Value::truthy);
        let mut value = base;
        for level in &levels {
            if let Some(declared_value) = level.get(k) {
                if declared_value.truthy() != base {
                    value = !value;
                }
            }
        }
        if let Some(direct) = input.direct.get(k) {
            value = direct.truthy();
        }
        layer.insert(k.clone(), Value::Bool(value));
    }
    layer
}
