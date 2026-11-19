//! Owns: a table style's formatting by region as docx-agent writes it
//! (`edit/table_styles.py`, round 58): the regions' names (and the other names read as
//! them), a region's fields checked (borders by side), each region written as its
//! `w:tblStylePr` (the whole table: the style's own properties) in Word's order, a band
//! size stated where a banded region needs one, and a border in the words a region reports
//! it in.
//!
//! Must not: decide which tables show a region (their look is theirs).

use super::shading::{attributes, set_shd};
use super::sheet::{make, Sheet};
use super::invalid;
use crate::format::{properties_of, set_child, write_paragraph, write_run, Property, Setting, Value};
use crate::names::W;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_base::pyfmt::repr_str;
use ooxml_xml::Element;

/// The regions as the tools name them and their `w:tblStylePr` types.
pub(crate) const KINDS: &[(&str, Option<&str>)] = &[
    ("whole_table", None),
    ("header_row", Some("firstRow")),
    ("total_row", Some("lastRow")),
    ("first_column", Some("firstCol")),
    ("last_column", Some("lastCol")),
    ("banded_rows", Some("band1Horz")),
    ("even_banded_rows", Some("band2Horz")),
    ("banded_columns", Some("band1Vert")),
    ("even_banded_columns", Some("band2Vert")),
    ("top_left_cell", Some("nwCell")),
    ("top_right_cell", Some("neCell")),
    ("bottom_left_cell", Some("swCell")),
    ("bottom_right_cell", Some("seCell")),
];

/// Other names a region is called by (look's, Word's Modify Style list's), read as it.
const ALIASES: &[(&str, &str)] = &[
    ("last_row", "total_row"),
    ("first_row", "header_row"),
    ("odd_banded_rows", "banded_rows"),
    ("odd_banded_columns", "banded_columns"),
    ("odd_rows", "banded_rows"),
    ("even_rows", "even_banded_rows"),
    ("odd_columns", "banded_columns"),
    ("even_columns", "even_banded_columns"),
    ("table", "whole_table"),
    ("firstRow", "header_row"),
    ("lastRow", "total_row"),
    ("firstCol", "first_column"),
    ("lastCol", "last_column"),
    ("band1Horz", "banded_rows"),
    ("band2Horz", "even_banded_rows"),
    ("band1Vert", "banded_columns"),
    ("band2Vert", "even_banded_columns"),
    ("nwCell", "top_left_cell"),
    ("neCell", "top_right_cell"),
    ("swCell", "bottom_left_cell"),
    ("seCell", "bottom_right_cell"),
    ("wholeTable", "whole_table"),
];

/// The order a style's regions are written in (Word's own table styles' order).
const ORDER: &[&str] = &[
    "firstRow", "lastRow", "firstCol", "lastCol", "band1Vert", "band2Vert", "band1Horz", "band2Horz", "neCell",
    "nwCell", "seCell", "swCell",
];

/// A border's sides as the tools name them, and their elements.
pub const BORDER_SIDES: &[(&str, &str)] = &[
    ("top", "top"),
    ("bottom", "bottom"),
    ("left", "left"),
    ("right", "right"),
    ("inside_h", "insideH"),
    ("inside_v", "insideV"),
];
/// A region border's styles.
pub const BORDER_STYLES: &[&str] = &["none", "single", "double", "thick", "dotted", "dashed"];

/// A region's borders as given.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegionBorders {
    /// The style of the sides given.
    pub style: Option<String>,
    /// The sides (default all).
    pub sides: Vec<String>,
    /// RRGGBB or auto.
    pub color: Option<String>,
    /// Points.
    pub width: Option<f64>,
}

/// One region's fields as given.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegionValues {
    /// The region's name as given.
    pub region: String,
    /// Fill.
    pub shading: Option<String>,
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
    /// Borders.
    pub borders: Option<RegionBorders>,
    /// Rows or columns per band.
    pub band_size: Option<i64>,
    /// Remove its formatting first.
    pub clear: Option<bool>,
}

impl RegionValues {
    /// Whether it sets anything (a `clear` alone sets nothing).
    pub(crate) fn sets(&self) -> bool {
        self.shading.is_some()
            || self.color.is_some()
            || self.bold.is_some()
            || self.italic.is_some()
            || self.size.is_some()
            || self.alignment.is_some()
            || self.borders.as_ref().is_some_and(|b| !b.is_empty())
            || self.band_size.is_some()
    }
}

impl RegionBorders {
    fn is_empty(&self) -> bool {
        self.style.is_none() && self.sides.is_empty() && self.color.is_none() && self.width.is_none()
    }
}

/// A region checked: its name, the note where it was read as another, its sides' styles.
#[derive(Debug, Clone)]
pub(crate) struct Checked {
    pub(crate) region: &'static str,
    pub(crate) values: RegionValues,
    pub(crate) sides: Vec<(&'static str, String)>,
}

fn region_names() -> Vec<String> {
    KINDS.iter().map(|(n, _)| (*n).to_owned()).collect()
}

/// `(region, note)`: the region `given` names (`region_name`).
fn region_name(given: &str) -> Result<(&'static str, Option<String>)> {
    let key = given.trim();
    if let Some((name, _)) = KINDS.iter().find(|(n, _)| *n == key) {
        return Ok((name, None));
    }
    let spaced = key.to_lowercase().replace([' ', '-'], "_");
    let found = KINDS
        .iter()
        .find(|(n, _)| *n == spaced)
        .map(|(n, _)| *n)
        .or_else(|| ALIASES.iter().find(|(a, _)| *a == key).map(|(_, n)| *n))
        .or_else(|| ALIASES.iter().find(|(a, _)| *a == spaced).map(|(_, n)| *n));
    let Some(found) = found else {
        let names: Vec<&str> = KINDS.iter().map(|(n, _)| *n).collect();
        return Err(invalid(
            format!(
                "no table style region {}: the regions are {} (a middle column is none of them: give the column cell formatting, or copy a table's frame with word_insert_markdown like)",
                repr_str(given),
                names.join(", ")
            ),
            Some("regions[].region"),
            region_names(),
        ));
    };
    let name = KINDS.iter().find(|(n, _)| *n == found).map_or("whole_table", |(n, _)| *n);
    Ok((name, Some(format!("region {} read as {name}", repr_str(given)))))
}

fn styles_named() -> Vec<String> {
    BORDER_STYLES.iter().map(|s| (*s).to_owned()).collect()
}

/// A region's borders by side (`borders_by_side`).
fn sides(borders: &RegionBorders) -> Result<Vec<(&'static str, String)>> {
    if borders.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    if borders.style.is_some() || !borders.sides.is_empty() {
        let Some(style) = &borders.style else {
            return Err(invalid(
                "borders: give the style for its sides".to_owned(),
                Some("regions[].borders.style"),
                styles_named(),
            ));
        };
        let given: Vec<String> = if borders.sides.is_empty() {
            BORDER_SIDES.iter().map(|(s, _)| (*s).to_owned()).collect()
        } else {
            borders.sides.clone()
        };
        for side in given {
            let Some((name, _)) = BORDER_SIDES.iter().find(|(s, _)| *s == side) else {
                return Err(invalid(
                    format!("no border side {}", repr_str(&side)),
                    Some("regions[].borders.sides"),
                    BORDER_SIDES.iter().map(|(s, _)| (*s).to_owned()).collect(),
                ));
            };
            if !BORDER_STYLES.contains(&style.as_str()) {
                return Err(invalid(
                    format!("border {side}: {} is no style", repr_str(style)),
                    Some("regions[].borders.style"),
                    styles_named(),
                ));
            }
            match out.iter_mut().find(|(s, _): &&mut (&str, String)| s == name) {
                Some(slot) => slot.1.clone_from(style),
                None => out.push((*name, style.clone())),
            }
        }
    }
    if let Some(color) = &borders.color {
        let hex = color.trim().len() == 6 && color.trim().chars().all(|c| c.is_ascii_hexdigit());
        if !(color.to_lowercase() == "auto" || hex) {
            return Err(invalid(
                "border color is RRGGBB or auto".to_owned(),
                Some("regions[].borders.color"),
                Vec::new(),
            ));
        }
    }
    if borders.width.is_some_and(|w| !(0.25..=6.0).contains(&w)) {
        return Err(invalid(
            "border width is in points, 0.25 to 6".to_owned(),
            Some("regions[].borders.width"),
            Vec::new(),
        ));
    }
    Ok(out)
}

/// `regions` read and checked, in order, and the notes on names read as others
/// (`_region_items`).
///
/// # Errors
/// A region or border given wrong.
pub(crate) fn check(regions: &[RegionValues]) -> Result<(Vec<Checked>, Vec<String>)> {
    let mut out = Vec::new();
    let mut notes = Vec::new();
    for entry in regions {
        if entry.region.is_empty() {
            return Err(EditError::Refused(
                "a region is {region, shading, color, bold, italic, size, alignment, borders, band_size, clear}"
                    .to_owned(),
            ));
        }
        let (region, note) = region_name(&entry.region)?;
        notes.extend(note);
        let sides = entry.borders.as_ref().map(sides).transpose()?.unwrap_or_default();
        out.push(Checked {
            region,
            values: entry.clone(),
            sides,
        });
    }
    Ok((out, notes))
}

fn kind_of(region: &str) -> Option<&'static str> {
    KINDS.iter().find(|(n, _)| *n == region).and_then(|(_, k)| *k)
}

fn rank(kind: Option<&str>) -> usize {
    kind.and_then(|k| ORDER.iter().position(|o| *o == k)).unwrap_or(ORDER.len())
}

/// The style's `w:tblStylePr` of `kind` (`None`: the style itself), made in Word's order.
fn style_part(work: &mut Work, node: Id, kind: Option<&str>, create: bool, orders: &Orders) -> Option<Id> {
    let Some(kind) = kind else { return Some(node) };
    let parts: Vec<Id> = work
        .children(node)
        .iter()
        .copied()
        .filter(|c| work.is(*c, W, "tblStylePr"))
        .collect();
    if let Some(found) = parts.iter().copied().find(|p| work.get(*p, Some(W), "type") == Some(kind)) {
        return Some(found);
    }
    if !create {
        return None;
    }
    let made = make(work, "tblStylePr", &[("type", kind)]);
    let mine = rank(Some(kind));
    match parts.into_iter().find(|p| rank(work.get(*p, Some(W), "type")) > mine) {
        Some(later) => work.add_previous(later, made),
        None => crate::marks::insert_in_order(work, node, made, orders),
    }
    Some(made)
}

fn child(work: &mut Work, owner: Id, local: &str, create: bool, orders: &Orders) -> Option<Id> {
    properties_of(work, owner, local, create, orders)
}

/// Empty property holders of a region removed (`_tidy`).
fn tidy(work: &mut Work, owner: Id) {
    for local in ["pPr", "rPr", "tcPr"] {
        if let Some(node) = work.find(owner, W, local) {
            if work.children(node).is_empty() && work.attributes(node).is_empty() {
                work.remove(node);
            }
        }
    }
    let borders = work.find(owner, W, "tcPr").and_then(|c| work.find(c, W, "tcBorders"));
    if let Some(borders) = borders.filter(|b| work.children(*b).is_empty()) {
        let parent = work.parent(borders);
        work.remove(borders);
        if let Some(parent) = parent.filter(|p| work.children(*p).is_empty()) {
            work.remove(parent);
        }
    }
}

fn run_settings(values: &RegionValues) -> Vec<Setting> {
    let mut out = Vec::new();
    if let Some(c) = &values.color {
        out.push((Property::Color, Value::Text(c.clone())));
    }
    if let Some(b) = values.bold {
        out.push((Property::Bold, Value::Flag(b)));
    }
    if let Some(i) = values.italic {
        out.push((Property::Italic, Value::Flag(i)));
    }
    if let Some(s) = values.size {
        out.push((Property::Size, Value::Number(s)));
    }
    out.into_iter()
        .map(|(property, value)| Setting {
            property,
            value: Some(value),
        })
        .collect()
}

/// Write `region`'s values on the table style `node` (`write_region`); returns notes.
///
/// # Errors
/// A value Word would not take.
pub(crate) fn write_region(work: &mut Work, node: Id, checked: &Checked, sheet: &Sheet) -> Result<Vec<String>> {
    let orders = sheet.orders();
    let mut notes = Vec::new();
    let region = checked.region;
    let values = &checked.values;
    let kind = kind_of(region);
    if values.clear == Some(true) {
        clear(work, node, kind, orders);
    }
    if !values.sets() {
        return Ok(notes);
    }
    let Some(part) = style_part(work, node, kind, true, orders) else {
        return Ok(notes);
    };
    let fail = |e: EditError| EditError::Refused(format!("{region}: {e}"));
    let run = run_settings(values);
    if !run.is_empty() {
        crate::format::check(&run).map_err(fail)?;
        let colour = |name: &str| sheet.colours.value(name);
        write_run(work, part, &run, &colour, orders);
    }
    if let Some(alignment) = &values.alignment {
        let setting = [Setting {
            property: Property::Alignment,
            value: Some(Value::Text(alignment.clone())),
        }];
        crate::format::check(&setting).map_err(fail)?;
        write_paragraph(work, part, &setting, orders);
    }
    if let Some(fill) = &values.shading {
        let shd = if fill.trim().eq_ignore_ascii_case("none") {
            None
        } else {
            Some(attributes(fill, &sheet.colours).map_err(fail)?)
        };
        if let Some(cell) = child(work, part, "tcPr", shd.is_some(), orders) {
            set_shd(work, cell, shd.as_ref(), orders);
        }
    }
    if !checked.sides.is_empty() {
        write_borders(work, part, kind, &checked.sides, values.borders.as_ref(), orders);
    }
    if let Some(size) = values.band_size {
        let band = match region {
            "banded_rows" | "even_banded_rows" => Some("tblStyleRowBandSize"),
            "banded_columns" | "even_banded_columns" => Some("tblStyleColBandSize"),
            _ => None,
        };
        match band {
            None => notes.push(format!("band_size is the banded regions': left out of {region}")),
            Some(band) => {
                if let Some(table) = child(work, node, "tblPr", true, orders) {
                    set_child(work, table, band, Some(&[("val", Some(&size.to_string()))]), orders);
                }
            }
        }
    }
    tidy(work, part);
    if kind.is_some() && work.parent(part).is_some() && work.children(part).is_empty() {
        work.remove(part);
    }
    Ok(notes)
}

/// What a region states removed (`clear`).
fn clear(work: &mut Work, node: Id, kind: Option<&str>, orders: &Orders) {
    if kind.is_some() {
        if let Some(found) = style_part(work, node, kind, false, orders) {
            work.remove(found);
        }
        return;
    }
    for local in ["pPr", "rPr", "tcPr"] {
        if let Some(found) = work.find(node, W, local) {
            work.remove(found);
        }
    }
    if let Some(table) = work.find(node, W, "tblPr") {
        for local in ["tblBorders", "shd"] {
            if let Some(found) = work.find(table, W, local) {
                work.remove(found);
            }
        }
    }
}

/// A region's borders, each side given replaced (`_write_borders`).
fn write_borders(
    work: &mut Work,
    part: Id,
    kind: Option<&str>,
    sides: &[(&str, String)],
    given: Option<&RegionBorders>,
    orders: &Orders,
) {
    let (outer, inner) = if kind.is_none() { ("tblPr", "tblBorders") } else { ("tcPr", "tcBorders") };
    let Some(outer) = child(work, part, outer, true, orders) else { return };
    let Some(holder) = child(work, outer, inner, true, orders) else { return };
    let color = given.and_then(|b| b.color.as_deref());
    let width = given.and_then(|b| b.width);
    for (side, local) in BORDER_SIDES {
        let Some((_, style)) = sides.iter().find(|(s, _)| s == side) else {
            continue;
        };
        let new = if style == "none" {
            make(work, local, &[("val", "nil")])
        } else {
            let size = (width.unwrap_or(0.5) * 8.0).round_ties_even() as i64;
            let colour = match color {
                Some(c) if !c.eq_ignore_ascii_case("auto") => c.to_uppercase(),
                _ => "auto".to_owned(),
            };
            make(
                work,
                local,
                &[("val", style), ("sz", &size.to_string()), ("space", "0"), ("color", &colour)],
            )
        };
        match work.find(holder, W, local) {
            Some(old) => {
                work.add_previous(old, new);
                work.remove(old);
            }
            None => crate::marks::insert_in_order(work, holder, new, orders),
        }
    }
}

/// A banded region drawn only where its chain states a band size: 1 written on the style
/// where none is (`ensure_band_sizes`). Returns notes.
pub(crate) fn ensure_band_sizes(work: &mut Work, root: Id, node: Id, regions: &[&str], orders: &Orders) -> Vec<String> {
    let mut notes = Vec::new();
    for (local, names) in [
        ("tblStyleRowBandSize", ["banded_rows", "even_banded_rows"]),
        ("tblStyleColBandSize", ["banded_columns", "even_banded_columns"]),
    ] {
        if !regions.iter().any(|r| names.contains(r)) {
            continue;
        }
        let mut current = Some(node);
        let mut seen = Vec::new();
        let mut stated = false;
        while let Some(at) = current.filter(|c| !seen.contains(c)) {
            seen.push(at);
            if work.find(at, W, "tblPr").and_then(|t| work.find(t, W, local)).is_some() {
                stated = true;
                break;
            }
            current = super::sheet::child_val(work, at, "basedOn")
                .map(str::to_owned)
                .and_then(|b| super::sheet::by_id(work, root, &b));
        }
        if !stated {
            if let Some(table) = child(work, node, "tblPr", true, orders) {
                set_child(work, table, local, Some(&[("val", Some("1"))]), orders);
            }
            let what = if local.contains("Row") { "rows" } else { "columns" };
            notes.push(format!(
                "{what} banded one at a time (band_size 1): a style states a band size for Word to draw its bands"
            ));
        }
    }
    notes
}

/// A border as a region reports it: `none`, or `style width pt colour` (`_border_words`).
#[must_use]
pub fn border_words(border: &Element) -> String {
    let style = border.attribute(Some(W), "val");
    if matches!(style, None | Some("nil" | "none")) {
        return "none".to_owned();
    }
    // `_int(node, "sz", 0) / 8`, 0.5 where it states none
    let size = border.attribute(Some(W), "sz").map_or(0.5, |s| {
        ooxml_word_model::xmlread::py_int(s).map_or(0.0, |s| s as f64 / 8.0)
    });
    let size = format!("{size:.3}");
    let size = size.trim_end_matches('0').trim_end_matches('.');
    let mut out = format!("{} {size} pt", style.unwrap_or(""));
    let colour = border
        .attribute(Some(W), "themeColor")
        .filter(|c| !c.is_empty())
        .or_else(|| border.attribute(Some(W), "color"));
    if let Some(colour) = colour.filter(|c| !c.is_empty() && *c != "auto") {
        out.push(' ');
        out.push_str(colour);
    }
    out
}
